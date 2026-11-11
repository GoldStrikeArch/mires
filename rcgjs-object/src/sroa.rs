//! Aggregates that are built only to be taken apart again.
//!
//! A value in this model is a JavaScript object or array, so a Rust value that is copied, or
//! handed to an operation that reads it field by field, costs an allocation. Where the source of
//! such a copy is a literal the emitter *just* built, the aggregate never has to exist: the copy
//! of a fresh literal is that literal, and an operation that reads a fresh literal's elements in
//! order can read the element expressions instead.
//!
//! A local the emitter fills with a literal and then only ever reads one field of at a time is the
//! same story spread over a function: `split_locals` gives each field a local of its own, and the
//! object is never built.
//!
//! The rewrites are exact by construction rather than by analysis. A literal is a fresh value that
//! nothing else can hold, so copying it is unobservable; substituting an argument for a parameter
//! each of whose uses is one indexed read, visited in the order JavaScript evaluates them,
//! evaluates every element exactly once in the order the array literal would have; and one local
//! per field holds the same values under the same conditions as the object it replaces. On the JIR
//! path, a direct alias of a dominating SSA definition can read that stable compiler slot directly.
//!
//! The pass runs on the item's finished declaration, like [`crate::optimizer`], and only when
//! [`OptimizerPass::Sroa`] is selected; report mode walks the same rewrite and returns the
//! original, which is what keeps `js-opt=off` and `js-opt=report` byte identical.

use std::collections::{BTreeMap, BTreeSet};

use crate::jsast::{self, ArrowBody, BinOp, Expr, Stmt, SwitchCase};
use crate::opts::{OptimizerMode, OptimizerPass};

pub struct Analysis {
    pub decl: Stmt,
    pub changed: bool,
}
/// Replaces the aggregates of `decl` that are consumed where they are built.
pub fn analyze(decl: Stmt, mode: OptimizerMode) -> Analysis {
    if mode == OptimizerMode::Off
        || !crate::opts::get()
            .optimizer_passes
            .contains(OptimizerPass::Sroa)
    {
        return Analysis {
            decl,
            changed: false,
        };
    }

    let rewritten = split_locals(rewrite_stmt(decl.clone()), false);
    let changed = rewritten != decl;
    Analysis {
        decl: if mode == OptimizerMode::On {
            rewritten
        } else {
            decl
        },
        changed,
    }
}

/// Runs the finished-AST rewrite for a JIR-lowered declaration.
///
/// JIR structuring can expose a product one layer at a time: splitting an iterator wrapper makes
/// its inner iterator a new local aggregate. The sysroot producer deliberately keeps the
/// historical one-round policy in [`analyze`]; the linker may peel up to eight layers and may
/// forward compiler-local aliases whose source value is never replaced.
pub fn analyze_jir(decl: Stmt, mode: OptimizerMode) -> Analysis {
    if mode == OptimizerMode::Off
        || !crate::opts::get()
            .optimizer_passes
            .contains(OptimizerPass::Sroa)
    {
        return Analysis {
            decl,
            changed: false,
        };
    }

    let original = decl;
    let mut rewritten = rewrite_stmt(original.clone());
    let start_cost = sroa_cost(&rewritten);
    let mut best = rewritten.clone();
    let mut best_cost = start_cost;
    for _ in 0..8 {
        let next = split_locals(forward_stable_aliases(rewritten.clone()), true);
        if next == rewritten {
            break;
        }
        let next_cost = sroa_cost(&next);
        if next_cost.compact < best_cost.compact
            || (next_cost.in_loop == 0
                && start_cost.in_loop > 0
                && next_cost.compact <= start_cost.compact.saturating_add(256)
                && best_cost.in_loop > 0)
        {
            best = next.clone();
            best_cost = next_cost;
        }
        rewritten = next;
    }
    rewritten = best;
    let changed = rewritten != original;
    Analysis {
        decl: if mode == OptimizerMode::On {
            rewritten
        } else {
            original
        },
        changed,
    }
}

/// Compact size and hot allocation sites for one hypothetical split round.
///
/// The extended JIR splitter can exchange one object literal for several lane writes. Doing that
/// everywhere is a wire-size regression even though each local transformation scalarizes. The
/// normal decision therefore keeps the smallest compact round. Completing an allocation-free
/// loop may spend at most 256 local bytes; partial loop scalarization gets no such exception.
#[derive(Clone, Copy)]
struct SroaCost {
    compact: u64,
    in_loop: u64,
}

fn sroa_cost(decl: &Stmt) -> SroaCost {
    let mut measured = decl.clone();
    crate::minify::shorten_locals_for_measurement(&mut measured);
    let bytes = jsast::program_to_string_styled_for_target(
        std::slice::from_ref(&measured),
        jsast::Style::Compact,
        crate::opts::get().ecma_target,
    )
    .len() as u64;
    SroaCost {
        compact: bytes,
        in_loop: FreshAllocations::of(decl).in_loop,
    }
}

/// Allocation-site weight for a finished JavaScript declaration.
struct FreshAllocations {
    depth: u32,
    in_loop: u64,
}

impl FreshAllocations {
    fn of(stmt: &Stmt) -> FreshAllocations {
        let mut allocations = Self {
            depth: 0,
            in_loop: 0,
        };
        allocations.stmt(stmt);
        allocations
    }

    fn record(&mut self) {
        self.in_loop += u64::from(self.depth > 0);
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ExprStmt(expr) | Stmt::Throw(expr) => self.expr(expr),
            Stmt::Let(_, init) => {
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            Stmt::Lets(declarations) => {
                for (_, init) in declarations {
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
            Stmt::If(test, then, otherwise) => {
                self.expr(test);
                self.stmts(then);
                if let Some(otherwise) = otherwise {
                    self.stmts(otherwise);
                }
            }
            Stmt::Switch(discriminant, cases) => {
                self.expr(discriminant);
                for case in cases {
                    for test in &case.tests {
                        self.expr(test);
                    }
                    self.stmts(&case.body);
                }
            }
            Stmt::Labeled(_, inner) => self.stmt(inner),
            Stmt::LoopForever(_, body) => self.loop_body(None, body),
            Stmt::While(test, body) => self.loop_body(Some(test), body),
            Stmt::FunctionDecl { body, .. } => {
                let depth = self.depth;
                self.depth = 0;
                self.stmts(body);
                self.depth = depth;
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.stmts(body);
                self.stmts(handler);
            }
            Stmt::Block(body) => self.stmts(body),
            Stmt::Import { .. }
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn loop_body(&mut self, test: Option<&Expr>, body: &[Stmt]) {
        self.depth += 1;
        if let Some(test) = test {
            self.expr(test);
        }
        self.stmts(body);
        self.depth -= 1;
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Member(object, _) | Expr::Unary(_, object) => self.expr(object),
            Expr::Index(object, index)
            | Expr::Binary(_, object, index)
            | Expr::Assign(object, index)
            | Expr::CompoundAssign(_, object, index) => {
                self.expr(object);
                self.expr(index);
            }
            Expr::Call(callee, arguments) => {
                self.expr(callee);
                for argument in arguments {
                    self.expr(argument);
                }
            }
            Expr::New(callee, arguments) => {
                self.record();
                self.expr(callee);
                for argument in arguments {
                    self.expr(argument);
                }
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test);
                self.expr(consequent);
                self.expr(alternate);
            }
            Expr::Object(entries) => {
                self.record();
                for (_, value) in entries {
                    self.expr(value);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                if matches!(expr, Expr::Array(_)) {
                    self.record();
                }
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Arrow(_, body) => {
                self.record();
                let depth = self.depth;
                self.depth = 0;
                match body {
                    ArrowBody::Expr(body) => self.expr(body),
                    ArrowBody::Block(body) => self.stmts(body),
                }
                self.depth = depth;
            }
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
}

fn rewrite_stmt(stmt: Stmt) -> Stmt {
    match stmt {
        Stmt::ExprStmt(expr) => Stmt::ExprStmt(rewrite_expr(expr)),
        Stmt::Throw(expr) => Stmt::Throw(rewrite_expr(expr)),
        Stmt::Let(name, init) => Stmt::Let(name, init.map(rewrite_expr)),
        Stmt::Lets(declarations) => Stmt::Lets(
            declarations
                .into_iter()
                .map(|(name, init)| (name, init.map(rewrite_expr)))
                .collect(),
        ),
        Stmt::Const(name, init) => Stmt::Const(name, rewrite_expr(init)),
        Stmt::Return(value) => Stmt::Return(value.map(rewrite_expr)),
        Stmt::If(test, then, otherwise) => Stmt::If(
            rewrite_expr(test),
            rewrite_stmts(then),
            otherwise.map(rewrite_stmts),
        ),
        Stmt::Switch(disc, cases) => Stmt::Switch(
            rewrite_expr(disc),
            cases
                .into_iter()
                .map(|case| SwitchCase {
                    tests: case.tests.into_iter().map(rewrite_expr).collect(),
                    body: rewrite_stmts(case.body),
                })
                .collect(),
        ),
        Stmt::Labeled(label, body) => Stmt::Labeled(label, Box::new(rewrite_stmt(*body))),
        Stmt::LoopForever(label, body) => Stmt::LoopForever(label, rewrite_stmts(body)),
        Stmt::While(test, body) => Stmt::While(rewrite_expr(test), rewrite_stmts(body)),
        Stmt::FunctionDecl { name, params, body } => Stmt::FunctionDecl {
            name,
            params,
            body: rewrite_stmts(body),
        },
        Stmt::TryCatch {
            body,
            binding,
            handler,
        } => Stmt::TryCatch {
            body: rewrite_stmts(body),
            binding,
            handler: rewrite_stmts(handler),
        },
        Stmt::Block(body) => Stmt::Block(rewrite_stmts(body)),
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => stmt,
    }
}

fn rewrite_stmts(stmts: Vec<Stmt>) -> Vec<Stmt> {
    stmts.into_iter().map(rewrite_stmt).collect()
}

fn rewrite_expr(expr: Expr) -> Expr {
    let expr = match expr {
        Expr::Member(obj, name) => Expr::Member(Box::new(rewrite_expr(*obj)), name),
        Expr::Index(obj, index) => {
            Expr::Index(Box::new(rewrite_expr(*obj)), Box::new(rewrite_expr(*index)))
        }
        Expr::Call(callee, args) => Expr::Call(
            Box::new(rewrite_expr(*callee)),
            args.into_iter().map(rewrite_expr).collect(),
        ),
        Expr::New(callee, args) => Expr::New(
            Box::new(rewrite_expr(*callee)),
            args.into_iter().map(rewrite_expr).collect(),
        ),
        Expr::Unary(op, operand) => Expr::Unary(op, Box::new(rewrite_expr(*operand))),
        Expr::Binary(op, lhs, rhs) => Expr::Binary(
            op,
            Box::new(rewrite_expr(*lhs)),
            Box::new(rewrite_expr(*rhs)),
        ),
        Expr::Assign(lhs, rhs) => {
            Expr::Assign(Box::new(rewrite_expr(*lhs)), Box::new(rewrite_expr(*rhs)))
        }
        Expr::CompoundAssign(op, lhs, rhs) => Expr::CompoundAssign(
            op,
            Box::new(rewrite_expr(*lhs)),
            Box::new(rewrite_expr(*rhs)),
        ),
        Expr::Cond(test, consequent, alternate) => Expr::Cond(
            Box::new(rewrite_expr(*test)),
            Box::new(rewrite_expr(*consequent)),
            Box::new(rewrite_expr(*alternate)),
        ),
        Expr::Object(entries) => Expr::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, rewrite_expr(value)))
                .collect(),
        ),
        Expr::Array(items) => Expr::Array(items.into_iter().map(rewrite_expr).collect()),
        Expr::Arrow(params, ArrowBody::Expr(body)) => {
            Expr::Arrow(params, ArrowBody::Expr(Box::new(rewrite_expr(*body))))
        }
        Expr::Arrow(params, ArrowBody::Block(body)) => {
            Expr::Arrow(params, ArrowBody::Block(rewrite_stmts(body)))
        }
        Expr::Seq(items) => Expr::Seq(items.into_iter().map(rewrite_expr).collect()),
        other => other,
    };
    forward_elements(drop_literal_copy(expr))
}

/// `[a, b].slice()` is `[a, b]`: a copy of a value nothing else holds.
///
/// `Array.prototype.slice` with no arguments answers a fresh array with the same elements, which
/// is what the literal already is. This is how a `[T; N]` operand reaches an operation that takes
/// its argument by value ([`crate::value::clone_expr`]).
fn drop_literal_copy(expr: Expr) -> Expr {
    let Expr::Call(callee, args) = &expr else {
        return expr;
    };
    if !args.is_empty() {
        return expr;
    }
    let Expr::Member(receiver, name) = callee.as_ref() else {
        return expr;
    };
    if name != "slice" || !matches!(receiver.as_ref(), Expr::Array(_)) {
        return expr;
    }
    let Expr::Call(callee, _) = expr else {
        unreachable!("the callee shape was matched above")
    };
    match *callee {
        Expr::Member(receiver, _) => *receiver,
        _ => unreachable!("the callee shape was matched above"),
    }
}

/// `(($b) => $b[0] | $b[1] << 8)([x, y])` is `x | y << 8`.
///
/// The emitted byte conversions ([`crate::intrinsics`]) bind their operand to a parameter so that
/// it is evaluated once, then read it element by element. When the operand is a literal there is
/// nothing to bind: substituting the elements evaluates each of them exactly once, and
/// [`indexed_reads_in_order`] is what establishes that the order is the one the literal would have
/// evaluated them in.
fn forward_elements(expr: Expr) -> Expr {
    let Expr::Call(callee, args) = &expr else {
        return expr;
    };
    let [Expr::Array(items)] = args.as_slice() else {
        return expr;
    };
    let Expr::Arrow(params, ArrowBody::Expr(body)) = callee.as_ref() else {
        return expr;
    };
    let [param] = params.as_slice() else {
        return expr;
    };
    let mut reads = Vec::new();
    if !indexed_reads_in_order(body, param, &mut reads) {
        return expr;
    }
    if reads.len() != items.len() || reads.iter().copied().ne(0..items.len()) {
        return expr;
    }
    let mut items = items.clone();
    let mut body = body.as_ref().clone();
    substitute(&mut body, param, &mut items.iter_mut().map(Some).collect());
    body
}

/// Whether every use of `param` in `expr` is one indexed read, appending the indices in the order
/// JavaScript evaluates them.
///
/// Returns `false` for any syntax whose evaluation order this walk does not model: a call, a
/// conditional, a short circuit operator, an assignment, or verbatim JavaScript. A `false` answer
/// only declines the rewrite.
fn indexed_reads_in_order(expr: &Expr, param: &str, reads: &mut Vec<usize>) -> bool {
    if let Expr::Index(obj, index) = expr {
        if matches!(obj.as_ref(), Expr::Ident(name) if name == param) {
            let Expr::Num(index) = index.as_ref() else {
                return false;
            };
            if *index < 0.0 || index.fract() != 0.0 {
                return false;
            }
            reads.push(*index as usize);
            return true;
        }
    }
    match expr {
        Expr::Ident(name) => name != param,
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
        Expr::Member(obj, _) => indexed_reads_in_order(obj, param, reads),
        Expr::Index(obj, index) => {
            indexed_reads_in_order(obj, param, reads) && indexed_reads_in_order(index, param, reads)
        }
        Expr::Unary(_, operand) => indexed_reads_in_order(operand, param, reads),
        Expr::Binary(BinOp::And | BinOp::Or, ..) => false,
        Expr::Binary(_, lhs, rhs) => {
            indexed_reads_in_order(lhs, param, reads) && indexed_reads_in_order(rhs, param, reads)
        }
        Expr::Object(entries) => entries
            .iter()
            .all(|(_, value)| indexed_reads_in_order(value, param, reads)),
        Expr::Array(items) | Expr::Seq(items) => items
            .iter()
            .all(|item| indexed_reads_in_order(item, param, reads)),
        Expr::Call(..)
        | Expr::New(..)
        | Expr::Assign(..)
        | Expr::CompoundAssign(..)
        | Expr::Cond(..)
        | Expr::Arrow(..)
        | Expr::Raw(_) => false,
    }
}

/// Replaces each `param[i]` in `expr` with the `i`th replacement, taking it as it goes.
///
/// Only called after [`indexed_reads_in_order`] has established that every use is one such read
/// and that the indices are `0..n` in evaluation order, so each replacement is taken exactly once.
fn substitute(expr: &mut Expr, param: &str, replacements: &mut Vec<Option<&mut Expr>>) {
    if let Expr::Index(obj, index) = expr {
        if matches!(obj.as_ref(), Expr::Ident(name) if name == param) {
            let Expr::Num(index) = index.as_ref() else {
                return;
            };
            let slot = &mut replacements[*index as usize];
            if let Some(replacement) = slot.take() {
                *expr = replacement.clone();
            }
            return;
        }
    }
    match expr {
        Expr::Member(obj, _) | Expr::Unary(_, obj) => substitute(obj, param, replacements),
        Expr::Index(lhs, rhs) | Expr::Binary(_, lhs, rhs) => {
            substitute(lhs, param, replacements);
            substitute(rhs, param, replacements);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                substitute(value, param, replacements);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                substitute(item, param, replacements);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FixedPath {
    root: String,
    fields: Vec<String>,
}

#[derive(Default)]
struct AliasFacts {
    whole_writes: BTreeMap<String, usize>,
    literal_writes: BTreeMap<String, usize>,
    aliases: BTreeMap<String, Vec<FixedPath>>,
    member_writes: BTreeSet<String>,
    shadowed: BTreeSet<String>,
    opaque: bool,
}

/// Forwards stable aliases of compiler-built values.
///
/// JIR SSA ensures the source definition dominates each alias. A direct alias of a once-written
/// compiler local is therefore the same JavaScript value, whether that value is scalar or has
/// identity. Mutation through an object alias remains visible. A field snapshot is narrower:
/// its root must be a private literal with only field uses, and its field must never be replaced.
/// Unknown calls, computed writes, and escaping aliases can mutate fields without a named store
/// in this body. Nested paths need a separate proof for each intermediate object.
fn forward_stable_aliases(decl: Stmt) -> Stmt {
    const COPY_COMPONENT_MIN_COMPACT_SAVING: u64 = 32;

    let Stmt::FunctionDecl { name, params, body } = decl else {
        return decl;
    };
    let locals = declared_locals(&body);
    let mut facts = AliasFacts::default();
    collect_alias_stmts(&body, &mut facts);
    if facts.opaque {
        return Stmt::FunctionDecl { name, params, body };
    }
    let mut uses: BTreeMap<String, LocalUse> = locals
        .iter()
        .map(|local| (local.clone(), LocalUse::default()))
        .collect();
    collect_stmts(&body, &mut uses);
    let private_shapes = split_shapes(&uses, true);

    let stable_roots: BTreeSet<String> = locals
        .iter()
        .filter(|local| {
            !facts.shadowed.contains(*local)
                && facts.whole_writes.get(*local) == Some(&1)
                && facts.literal_writes.get(*local) == Some(&1)
        })
        .cloned()
        .collect();
    let stable_values: BTreeSet<String> = locals
        .iter()
        .filter(|local| {
            !facts.shadowed.contains(*local)
                && facts.whole_writes.get(*local) == Some(&1)
                && !facts.aliases.contains_key(*local)
        })
        .cloned()
        .chain(
            params
                .iter()
                .filter(|param| {
                    !facts.shadowed.contains(*param) && !facts.whole_writes.contains_key(*param)
                })
                .cloned(),
        )
        .collect();
    let mut forwarded: BTreeMap<String, FixedPath> = BTreeMap::new();
    for _ in 0..facts.aliases.len() {
        let mut changed = false;
        for (alias, sources) in &facts.aliases {
            if forwarded.contains_key(alias)
                || !locals.contains(alias)
                || facts.shadowed.contains(alias)
                || facts.whole_writes.get(alias) != Some(&1)
            {
                continue;
            }
            let [source] = sources.as_slice() else {
                continue;
            };
            let canonical = if stable_roots.contains(&source.root)
                || (source.fields.is_empty() && stable_values.contains(&source.root))
            {
                source.clone()
            } else if let Some(parent) = forwarded.get(&source.root) {
                let mut canonical = parent.clone();
                canonical.fields.extend(source.fields.clone());
                canonical
            } else {
                continue;
            };
            if !canonical.fields.is_empty() {
                let [field] = canonical.fields.as_slice() else {
                    continue;
                };
                if !stable_roots.contains(&canonical.root)
                    || !private_shapes.contains_key(&canonical.root)
                    || !uses[&canonical.root]
                        .written
                        .iter()
                        .all(|fields| fields.contains(field))
                    || facts.member_writes.contains(field)
                {
                    continue;
                }
            }
            forwarded.insert(alias.clone(), canonical);
            changed = true;
        }
        if !changed {
            break;
        }
    }
    let direct = forwarded.clone();
    forward_copy_components(&locals, &facts, &stable_values, &mut forwarded);
    if forwarded.len() != direct.len() {
        let direct_cost = alias_compact_cost(&name, &params, &body, &direct);
        let component_cost = alias_compact_cost(&name, &params, &body, &forwarded);
        // Small raw substitutions can grow once the complete artifact is compressed.
        if direct_cost < component_cost.saturating_add(COPY_COMPONENT_MIN_COMPACT_SAVING) {
            forwarded = direct;
        }
    }
    if forwarded.is_empty() {
        return Stmt::FunctionDecl { name, params, body };
    }
    Stmt::FunctionDecl {
        name,
        params,
        body: rewrite_alias_stmts(body, &forwarded),
    }
}

/// Compact function bytes after substituting one alias set.
fn alias_compact_cost(
    name: &str,
    params: &[String],
    body: &[Stmt],
    aliases: &BTreeMap<String, FixedPath>,
) -> u64 {
    sroa_cost(&Stmt::FunctionDecl {
        name: name.to_owned(),
        params: params.to_vec(),
        body: rewrite_alias_stmts(body.to_vec(), aliases),
    })
    .compact
}

/// Adds locals whose every write copies one value from a single stable source.
///
/// Splitting a pointer record exposes invariant lanes such as `buf` and `sc`, but loop phis can
/// copy those lanes through a cycle. Every value in such a component remains equal to its one
/// external anchor. A component with a non-copy write, an unknown source, or multiple anchors is
/// kept.
fn forward_copy_components(
    locals: &BTreeSet<String>,
    facts: &AliasFacts,
    stable_values: &BTreeSet<String>,
    forwarded: &mut BTreeMap<String, FixedPath>,
) {
    let eligible: BTreeSet<String> = facts
        .aliases
        .iter()
        .filter(|(name, sources)| {
            locals.contains(*name)
                && !facts.shadowed.contains(*name)
                && facts.whole_writes.get(*name) == Some(&sources.len())
                && sources.iter().all(|source| source.fields.is_empty())
        })
        .map(|(name, _)| name.clone())
        .collect();
    let mut graph: BTreeMap<String, BTreeSet<String>> = eligible
        .iter()
        .map(|name| (name.clone(), BTreeSet::new()))
        .collect();
    for name in &eligible {
        for source in &facts.aliases[name] {
            if eligible.contains(&source.root) {
                graph
                    .get_mut(name)
                    .expect("eligible locals have graph entries")
                    .insert(source.root.clone());
                graph
                    .get_mut(&source.root)
                    .expect("eligible locals have graph entries")
                    .insert(name.clone());
            }
        }
    }

    let mut seen = BTreeSet::new();
    for start in &eligible {
        if !seen.insert(start.clone()) {
            continue;
        }
        let mut component = Vec::new();
        let mut work = vec![start.clone()];
        while let Some(name) = work.pop() {
            component.push(name.clone());
            for peer in &graph[&name] {
                if seen.insert(peer.clone()) {
                    work.push(peer.clone());
                }
            }
        }

        let mut anchors = BTreeSet::new();
        let mut valid = true;
        for name in &component {
            for source in &facts.aliases[name] {
                if eligible.contains(&source.root) {
                    continue;
                }
                if let Some(anchor) = forwarded.get(&source.root) {
                    anchors.insert(anchor.clone());
                } else if stable_values.contains(&source.root) {
                    anchors.insert(source.clone());
                } else {
                    valid = false;
                }
            }
        }
        if anchors.len() != 1 {
            continue;
        }
        let anchor = anchors
            .into_iter()
            .next()
            .expect("one anchor was established above");
        if valid {
            for name in component {
                forwarded.insert(name, anchor.clone());
            }
        }
    }
}

fn fixed_path(expr: &Expr) -> Option<FixedPath> {
    let mut fields = Vec::new();
    let mut current = expr;
    loop {
        match current {
            Expr::Ident(root) => {
                fields.reverse();
                return Some(FixedPath {
                    root: root.clone(),
                    fields,
                });
            }
            Expr::Member(object, field) => {
                fields.push(field.clone());
                current = object;
            }
            _ => return None,
        }
    }
}

fn collect_alias_stmts(stmts: &[Stmt], facts: &mut AliasFacts) {
    for stmt in stmts {
        collect_alias_stmt(stmt, facts);
    }
}

fn collect_alias_stmt(stmt: &Stmt, facts: &mut AliasFacts) {
    match stmt {
        Stmt::ExprStmt(expr) | Stmt::Throw(expr) => collect_alias_expr(expr, facts),
        Stmt::Let(name, init) => {
            if let Some(init) = init {
                note_binding(name, init, facts);
                collect_alias_expr(init, facts);
            }
        }
        Stmt::Lets(declarations) => {
            for (name, init) in declarations {
                if let Some(init) = init {
                    note_binding(name, init, facts);
                    collect_alias_expr(init, facts);
                }
            }
        }
        Stmt::Const(name, init) => {
            note_binding(name, init, facts);
            collect_alias_expr(init, facts);
        }
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_alias_expr(value, facts);
            }
        }
        Stmt::If(test, then, otherwise) => {
            collect_alias_expr(test, facts);
            collect_alias_stmts(then, facts);
            if let Some(otherwise) = otherwise {
                collect_alias_stmts(otherwise, facts);
            }
        }
        Stmt::Switch(disc, cases) => {
            collect_alias_expr(disc, facts);
            for case in cases {
                for test in &case.tests {
                    collect_alias_expr(test, facts);
                }
                collect_alias_stmts(&case.body, facts);
            }
        }
        Stmt::Labeled(_, body) => collect_alias_stmt(body, facts),
        Stmt::LoopForever(_, body) | Stmt::Block(body) => collect_alias_stmts(body, facts),
        Stmt::While(test, body) => {
            collect_alias_expr(test, facts);
            collect_alias_stmts(body, facts);
        }
        Stmt::FunctionDecl {
            name, params, body, ..
        } => {
            facts.shadowed.insert(name.clone());
            facts.shadowed.extend(params.iter().cloned());
            collect_alias_stmts(body, facts);
        }
        // A throw can expose a prefix of the assignments, and the catch binding can shadow
        // an outer alias. The straight-line alias proof does not cover either obligation.
        Stmt::Raw(_) | Stmt::TryCatch { .. } => facts.opaque = true,
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Loc(_) => {}
    }
}

fn note_binding(name: &str, value: &Expr, facts: &mut AliasFacts) {
    *facts.whole_writes.entry(name.to_owned()).or_default() += 1;
    if matches!(value, Expr::Object(_)) {
        *facts.literal_writes.entry(name.to_owned()).or_default() += 1;
    }
    if let Some(source) = fixed_path(value) {
        facts
            .aliases
            .entry(name.to_owned())
            .or_default()
            .push(source);
    }
}

fn collect_alias_expr(expr: &Expr, facts: &mut AliasFacts) {
    match expr {
        Expr::Assign(target, value) => {
            if let Some(path) = fixed_path(target) {
                if let Some(field) = path.fields.last() {
                    facts.member_writes.insert(field.clone());
                } else {
                    note_binding(&path.root, value, facts);
                }
            }
            collect_alias_expr(target, facts);
            collect_alias_expr(value, facts);
        }
        Expr::CompoundAssign(_, target, value) => {
            if let Some(path) = fixed_path(target) {
                if let Some(field) = path.fields.last() {
                    facts.member_writes.insert(field.clone());
                } else {
                    *facts.whole_writes.entry(path.root).or_default() += 1;
                }
            }
            collect_alias_expr(target, facts);
            collect_alias_expr(value, facts);
        }
        Expr::Member(object, _) | Expr::Unary(_, object) => collect_alias_expr(object, facts),
        Expr::Index(object, index) | Expr::Binary(_, object, index) => {
            collect_alias_expr(object, facts);
            collect_alias_expr(index, facts);
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            collect_alias_expr(callee, facts);
            for arg in args {
                collect_alias_expr(arg, facts);
            }
        }
        Expr::Cond(test, consequent, alternate) => {
            collect_alias_expr(test, facts);
            collect_alias_expr(consequent, facts);
            collect_alias_expr(alternate, facts);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                collect_alias_expr(value, facts);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                collect_alias_expr(item, facts);
            }
        }
        Expr::Arrow(params, body) => {
            facts.shadowed.extend(params.iter().cloned());
            match body {
                ArrowBody::Expr(body) => collect_alias_expr(body, facts),
                ArrowBody::Block(body) => collect_alias_stmts(body, facts),
            }
        }
        Expr::Raw(_) => facts.opaque = true,
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_) => {}
    }
}

fn path_expr(path: &FixedPath) -> Expr {
    path.fields
        .iter()
        .fold(jsast::id(path.root.clone()), |value, field| {
            jsast::member(value, field.clone())
        })
}

fn rewrite_alias_stmts(stmts: Vec<Stmt>, aliases: &BTreeMap<String, FixedPath>) -> Vec<Stmt> {
    stmts
        .into_iter()
        .map(|stmt| rewrite_alias_stmt(stmt, aliases))
        .filter(|stmt| !matches!(stmt, Stmt::ExprStmt(Expr::Undefined)))
        .collect()
}

fn rewrite_alias_stmt(stmt: Stmt, aliases: &BTreeMap<String, FixedPath>) -> Stmt {
    match stmt {
        Stmt::ExprStmt(expr) => Stmt::ExprStmt(rewrite_alias_expr(expr, aliases)),
        Stmt::Throw(expr) => Stmt::Throw(rewrite_alias_expr(expr, aliases)),
        Stmt::Let(name, init) => {
            Stmt::Let(name, init.map(|value| rewrite_alias_expr(value, aliases)))
        }
        Stmt::Lets(declarations) => Stmt::Lets(
            declarations
                .into_iter()
                .map(|(name, init)| (name, init.map(|value| rewrite_alias_expr(value, aliases))))
                .collect(),
        ),
        Stmt::Const(name, value) => Stmt::Const(name, rewrite_alias_expr(value, aliases)),
        Stmt::Return(value) => Stmt::Return(value.map(|value| rewrite_alias_expr(value, aliases))),
        Stmt::If(test, then, otherwise) => Stmt::If(
            rewrite_alias_expr(test, aliases),
            rewrite_alias_stmts(then, aliases),
            otherwise.map(|body| rewrite_alias_stmts(body, aliases)),
        ),
        Stmt::Switch(disc, cases) => Stmt::Switch(
            rewrite_alias_expr(disc, aliases),
            cases
                .into_iter()
                .map(|case| SwitchCase {
                    tests: case
                        .tests
                        .into_iter()
                        .map(|test| rewrite_alias_expr(test, aliases))
                        .collect(),
                    body: rewrite_alias_stmts(case.body, aliases),
                })
                .collect(),
        ),
        Stmt::Labeled(label, body) => {
            Stmt::Labeled(label, Box::new(rewrite_alias_stmt(*body, aliases)))
        }
        Stmt::LoopForever(label, body) => {
            Stmt::LoopForever(label, rewrite_alias_stmts(body, aliases))
        }
        Stmt::While(test, body) => Stmt::While(
            rewrite_alias_expr(test, aliases),
            rewrite_alias_stmts(body, aliases),
        ),
        Stmt::FunctionDecl { name, params, body } => Stmt::FunctionDecl {
            name,
            params,
            body: rewrite_alias_stmts(body, aliases),
        },
        Stmt::TryCatch { .. } => stmt,
        Stmt::Block(body) => Stmt::Block(rewrite_alias_stmts(body, aliases)),
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => stmt,
    }
}

fn rewrite_alias_expr(expr: Expr, aliases: &BTreeMap<String, FixedPath>) -> Expr {
    match expr {
        Expr::Ident(name) => aliases
            .get(&name)
            .map(path_expr)
            .unwrap_or(Expr::Ident(name)),
        Expr::Member(object, field) => jsast::member(rewrite_alias_expr(*object, aliases), field),
        Expr::Index(object, index) => Expr::Index(
            Box::new(rewrite_alias_expr(*object, aliases)),
            Box::new(rewrite_alias_expr(*index, aliases)),
        ),
        Expr::Call(callee, args) => Expr::Call(
            Box::new(rewrite_alias_expr(*callee, aliases)),
            args.into_iter()
                .map(|arg| rewrite_alias_expr(arg, aliases))
                .collect(),
        ),
        Expr::New(callee, args) => Expr::New(
            Box::new(rewrite_alias_expr(*callee, aliases)),
            args.into_iter()
                .map(|arg| rewrite_alias_expr(arg, aliases))
                .collect(),
        ),
        Expr::Unary(op, operand) => {
            Expr::Unary(op, Box::new(rewrite_alias_expr(*operand, aliases)))
        }
        Expr::Binary(op, left, right) => Expr::Binary(
            op,
            Box::new(rewrite_alias_expr(*left, aliases)),
            Box::new(rewrite_alias_expr(*right, aliases)),
        ),
        Expr::Assign(target, value) => {
            if matches!(target.as_ref(), Expr::Ident(name) if aliases.contains_key(name)) {
                Expr::Undefined
            } else {
                Expr::Assign(
                    Box::new(rewrite_alias_expr(*target, aliases)),
                    Box::new(rewrite_alias_expr(*value, aliases)),
                )
            }
        }
        Expr::CompoundAssign(op, target, value) => Expr::CompoundAssign(
            op,
            Box::new(rewrite_alias_expr(*target, aliases)),
            Box::new(rewrite_alias_expr(*value, aliases)),
        ),
        Expr::Cond(test, consequent, alternate) => Expr::Cond(
            Box::new(rewrite_alias_expr(*test, aliases)),
            Box::new(rewrite_alias_expr(*consequent, aliases)),
            Box::new(rewrite_alias_expr(*alternate, aliases)),
        ),
        Expr::Object(entries) => Expr::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, rewrite_alias_expr(value, aliases)))
                .collect(),
        ),
        Expr::Array(items) => Expr::Array(
            items
                .into_iter()
                .map(|item| rewrite_alias_expr(item, aliases))
                .collect(),
        ),
        Expr::Arrow(params, ArrowBody::Expr(body)) => Expr::Arrow(
            params,
            ArrowBody::Expr(Box::new(rewrite_alias_expr(*body, aliases))),
        ),
        Expr::Arrow(params, ArrowBody::Block(body)) => {
            Expr::Arrow(params, ArrowBody::Block(rewrite_alias_stmts(body, aliases)))
        }
        Expr::Seq(items) => {
            let mut items: Vec<Expr> = items
                .into_iter()
                .map(|item| rewrite_alias_expr(item, aliases))
                .filter(|item| !matches!(item, Expr::Undefined))
                .collect();
            match items.len() {
                0 => Expr::Undefined,
                1 => items.pop().expect("the sequence has one item"),
                _ => Expr::Seq(items),
            }
        }
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Raw(_) => expr,
    }
}

/// What one function-scoped local is used for, as far as splitting it is concerned.
#[derive(Default)]
struct LocalUse {
    /// The field names of each whole-value assignment, in the order the literal writes them.
    written: Vec<Vec<String>>,
    /// Other locals assigned to this local as a whole value. A connected group can be split
    /// together, turning the copy into one assignment per lane.
    copies: BTreeSet<String>,
    /// Other locals compared with this one by `__rt.ptr_eq`. Pointer identity is the scalar
    /// `buf`/`off`/`sc` tuple, so a connected record group can compare those lanes directly.
    pointer_eq: BTreeSet<String>,
    /// Whether an object assignment reads the previous value of this local one field at a time.
    /// This is safe to split, but belongs to the bounded small-product extension rather than the
    /// historical same-literal rule.
    self_update: bool,
    /// The field names read or written through the local.
    fields: BTreeSet<String>,
    /// Whether a field of the local is assigned in place: `local.field = ...` or
    /// `local.field op= ...`. Splitting turns a whole-local copy into one copy per lane, which
    /// means the same as the JavaScript assignment only while the shared object is never
    /// changed in place.
    stores_field: bool,
    /// Whether the local is used in any way that is not one whole-value assignment or one field.
    opaque: bool,
}

/// Splits a function's local aggregates into one local per field.
///
/// A local qualifies when every assignment to it is one object literal statement, all of those
/// literals write the same fields in the same order, and every other mention of it is one field of
/// that set. Assigning the fields one after another in the literal's own order then evaluates the
/// same expressions in the same order and leaves the same values readable under the same
/// conditions, with no object built.
///
/// A literal that mentions the local it is assigned to is rejected: the object would read the
/// previous value's fields, and separate assignments would already have overwritten some of them.
/// Only locals declared directly in the function body are considered, so a split never moves a
/// binding out of the block it belongs to.
fn split_locals(decl: Stmt, allow_extended: bool) -> Stmt {
    let Stmt::FunctionDecl { name, params, body } = decl else {
        return decl;
    };
    let mut uses: BTreeMap<String, LocalUse> = declared_locals(&body)
        .into_iter()
        .filter(|local| !params.contains(local))
        .map(|local| (local, LocalUse::default()))
        .collect();
    if uses.is_empty() {
        return Stmt::FunctionDecl { name, params, body };
    }
    for stmt in &body {
        collect_stmt(stmt, &mut uses);
    }

    let taken: BTreeSet<String> = identifiers(&body);
    let shapes = split_shapes(&uses, allow_extended);
    let mut split: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut allocated = taken.clone();
    for (local, fields) in shapes {
        if fields.is_empty() {
            continue;
        }
        let names = fields
            .iter()
            .map(|field| {
                let mut candidate = format!("{local}${field}");
                while allocated.contains(&candidate) {
                    candidate.push('$');
                }
                allocated.insert(candidate.clone());
                (field.clone(), candidate)
            })
            .collect();
        split.insert(local, names);
    }
    if split.is_empty() {
        return Stmt::FunctionDecl { name, params, body };
    }

    let body = body
        .into_iter()
        .map(|stmt| apply_stmt(stmt, &split))
        .collect();
    Stmt::FunctionDecl { name, params, body }
}

/// The lane fields of every splittable local.
///
/// Whole-local copies and pointer equality connect records whose split must agree. A component
/// is eligible only when all of its locals are otherwise field-only and at least one assignment
/// constructs an object literal. Variant objects may omit payload fields: a missing property and
/// an `undefined` lane are observationally equal under the field-only proof.
///
/// A whole-local assignment `x = y` does not copy anything: it makes `x` a second name for the
/// object `y` holds, and that is how the lowering spells a reference (`&mut S` is the object
/// itself), so an in-place store through one name is visible through the other. The split
/// turns that assignment into one copy per lane, which is the same thing only while the shared
/// object is never changed in place. A component in which a local on either side of a copy also
/// stores a field is therefore kept whole.
fn split_shapes(
    uses: &BTreeMap<String, LocalUse>,
    allow_extended: bool,
) -> BTreeMap<String, Vec<String>> {
    let aliased: BTreeSet<&String> = uses
        .iter()
        .flat_map(|(name, use_)| {
            use_.copies
                .iter()
                .filter(|source| uses.contains_key(*source))
                .flat_map(move |source| [name, source])
        })
        .collect();
    let mut graph: BTreeMap<String, BTreeSet<String>> = uses
        .keys()
        .map(|name| (name.clone(), BTreeSet::new()))
        .collect();
    for (name, use_) in uses {
        for peer in use_.copies.iter().chain(&use_.pointer_eq) {
            if !uses.contains_key(peer) {
                continue;
            }
            graph.entry(name.clone()).or_default().insert(peer.clone());
            graph.entry(peer.clone()).or_default().insert(name.clone());
        }
    }

    let mut out = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for root in uses.keys() {
        if !seen.insert(root.clone()) {
            continue;
        }
        let mut component = Vec::new();
        let mut work = vec![root.clone()];
        while let Some(name) = work.pop() {
            component.push(name.clone());
            for peer in &graph[&name] {
                if seen.insert(peer.clone()) {
                    work.push(peer.clone());
                }
            }
        }
        if component.iter().any(|name| uses[name].opaque)
            || !component.iter().any(|name| !uses[name].written.is_empty())
            || component
                .iter()
                .any(|name| uses[name].stores_field && aliased.contains(name))
        {
            continue;
        }
        let mut fields = BTreeSet::new();
        for name in &component {
            let use_ = &uses[name];
            fields.extend(use_.fields.iter().cloned());
            for written in &use_.written {
                fields.extend(written.iter().cloned());
            }
        }
        if fields.is_empty() || fields.iter().any(|field| !jsast::is_plain_ident(field)) {
            continue;
        }
        let fields: Vec<String> = fields.into_iter().collect();
        let historical = component.len() == 1
            && uses[&component[0]].copies.is_empty()
            && uses[&component[0]].pointer_eq.is_empty()
            && !uses[&component[0]].self_update
            && historical_shape(&uses[&component[0]]).is_some();
        if !historical && (!allow_extended || !small_extended_shape(&component, &fields, uses)) {
            continue;
        }
        for name in component {
            out.insert(name, fields.clone());
        }
    }
    out
}

/// Whether `use_` satisfies the original local-splitting rule.
fn historical_shape(use_: &LocalUse) -> Option<&[String]> {
    let (first, rest) = use_.written.split_first()?;
    if rest.iter().any(|fields| fields != first)
        || !use_.fields.iter().all(|field| first.contains(field))
        || first.iter().any(|field| !jsast::is_plain_ident(field))
    {
        return None;
    }
    Some(first)
}

/// Bounds the new copy, loop-update, and variant machinery to the physical shapes NS4 needs.
///
/// Pointer/slice records are the compiler's `buf`/`off` plus `len`/`sc` view shape. Other products
/// are deliberately capped at three lanes. A varying-field product must carry Rust's ordinary
/// `TAG` discriminant; an untagged product must write every lane on every construction. This keeps
/// broad records on the historical path and avoids trading one allocation for large amounts of
/// repeated lane code.
fn small_extended_shape(
    component: &[String],
    fields: &[String],
    uses: &BTreeMap<String, LocalUse>,
) -> bool {
    let pointer_or_slice = fields.iter().any(|field| field == "buf")
        && fields.iter().any(|field| field == "off")
        && fields
            .iter()
            .all(|field| matches!(field.as_str(), "buf" | "off" | "len" | "sc"));
    if pointer_or_slice {
        return true;
    }
    if fields.len() > 3 {
        return false;
    }

    let tagged = fields.iter().any(|field| field == "TAG");
    component.iter().all(|name| {
        uses[name].written.iter().all(|written| {
            if tagged {
                written.iter().any(|field| field == "TAG")
            } else {
                written.len() == fields.len() && fields.iter().all(|field| written.contains(field))
            }
        })
    })
}

/// The names a statement declares directly in the function body.
fn declared_locals(body: &[Stmt]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for stmt in body {
        match stmt {
            Stmt::Let(name, None) => {
                names.insert(name.clone());
            }
            Stmt::Lets(declarations) => {
                names.extend(
                    declarations
                        .iter()
                        .filter(|(_, init)| init.is_none())
                        .map(|(name, _)| name.clone()),
                );
            }
            _ => {}
        }
    }
    names
}

fn identifiers(body: &[Stmt]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for stmt in body {
        jsast::visit_idents(stmt, &mut |ident| {
            names.insert(ident.to_owned());
        });
    }
    names
}

fn collect_stmt(stmt: &Stmt, uses: &mut BTreeMap<String, LocalUse>) {
    match stmt {
        // A whole-value assignment standing on its own is the one shape a split rewrites;
        // `collect_expr` recognizes it, nested or not.
        Stmt::ExprStmt(expr) | Stmt::Throw(expr) => collect_expr(expr, uses),
        Stmt::Let(_, init) => {
            if let Some(init) = init {
                collect_expr(init, uses);
            }
        }
        Stmt::Lets(declarations) => {
            for (_, init) in declarations {
                if let Some(init) = init {
                    collect_expr(init, uses);
                }
            }
        }
        Stmt::Const(_, init) => collect_expr(init, uses),
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_expr(value, uses);
            }
        }
        Stmt::If(test, then, otherwise) => {
            collect_expr(test, uses);
            collect_stmts(then, uses);
            if let Some(otherwise) = otherwise {
                collect_stmts(otherwise, uses);
            }
        }
        Stmt::Switch(disc, cases) => {
            collect_expr(disc, uses);
            for case in cases {
                for test in &case.tests {
                    collect_expr(test, uses);
                }
                collect_stmts(&case.body, uses);
            }
        }
        Stmt::Labeled(_, body) => collect_stmt(body, uses),
        Stmt::LoopForever(_, body) | Stmt::Block(body) | Stmt::FunctionDecl { body, .. } => {
            collect_stmts(body, uses);
        }
        Stmt::While(test, body) => {
            collect_expr(test, uses);
            collect_stmts(body, uses);
        }
        // Verbatim JavaScript may name anything, so nothing in this function can be split.
        Stmt::Raw(_) | Stmt::TryCatch { .. } => {
            for use_ in uses.values_mut() {
                use_.opaque = true;
            }
        }
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Loc(_) => {}
    }
}

fn collect_stmts(stmts: &[Stmt], uses: &mut BTreeMap<String, LocalUse>) {
    for stmt in stmts {
        collect_stmt(stmt, uses);
    }
}

fn collect_expr(expr: &Expr, uses: &mut BTreeMap<String, LocalUse>) {
    match expr {
        Expr::Ident(name) => {
            if let Some(use_) = uses.get_mut(name) {
                use_.opaque = true;
            }
        }
        // One named field of the local, which a split turns into one local of its own.
        Expr::Member(obj, key) => {
            if let Expr::Ident(name) = obj.as_ref() {
                if let Some(use_) = uses.get_mut(name) {
                    use_.fields.insert(key.clone());
                    return;
                }
            }
            collect_expr(obj, uses);
        }
        Expr::Index(obj, index) => {
            collect_expr(obj, uses);
            collect_expr(index, uses);
        }
        Expr::Call(callee, args) if pointer_eq_args(callee, args).is_some() => {
            let (left, right) = pointer_eq_args(callee, args).expect("matched above");
            if uses.contains_key(left) && uses.contains_key(right) {
                for (name, peer) in [(left, right), (right, left)] {
                    let use_ = uses.get_mut(name).expect("checked above");
                    use_.fields
                        .extend(["buf", "off", "sc"].into_iter().map(str::to_owned));
                    use_.pointer_eq.insert(peer.to_owned());
                }
            } else {
                collect_expr(callee, uses);
                for arg in args {
                    collect_expr(arg, uses);
                }
            }
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            collect_expr(callee, uses);
            for arg in args {
                collect_expr(arg, uses);
            }
        }
        Expr::Unary(_, operand) => collect_expr(operand, uses),
        // A previous split spells the field writes as a comma expression. Recognize a nested
        // whole-local assignment too, so a JIR-only bounded retry can peel the next product layer
        // without changing the one-round policy used while building the sysroot.
        Expr::Assign(target, value) => {
            if collect_whole_assignment(target, value, uses) {
                return;
            }
            note_field_store(target, uses);
            collect_expr(target, uses);
            collect_expr(value, uses);
        }
        Expr::CompoundAssign(_, target, value) => {
            note_field_store(target, uses);
            collect_expr(target, uses);
            collect_expr(value, uses);
        }
        Expr::Binary(_, lhs, rhs) => {
            collect_expr(lhs, uses);
            collect_expr(rhs, uses);
        }
        Expr::Cond(test, consequent, alternate) => {
            collect_expr(test, uses);
            collect_expr(consequent, uses);
            collect_expr(alternate, uses);
        }
        Expr::Object(entries) => {
            for (_, value) in entries {
                collect_expr(value, uses);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for item in items {
                collect_expr(item, uses);
            }
        }
        Expr::Arrow(params, body) => {
            // A parameter of the same name shadows the local, so the body says nothing about it.
            for param in params {
                if let Some(use_) = uses.get_mut(param) {
                    use_.opaque = true;
                }
            }
            match body {
                ArrowBody::Expr(body) => collect_expr(body, uses),
                ArrowBody::Block(body) => collect_stmts(body, uses),
            }
        }
        Expr::Raw(_) => {
            for use_ in uses.values_mut() {
                use_.opaque = true;
            }
        }
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => {}
    }
}

/// Records that `target`, the left-hand side of an assignment, changes one field of a local in
/// place. A store into a field's own object (`local.field.inner = ...`) is not one: every lane
/// copy of `local.field` still names that object.
fn note_field_store(target: &Expr, uses: &mut BTreeMap<String, LocalUse>) {
    if let Expr::Member(object, _) = target {
        if let Expr::Ident(name) = object.as_ref() {
            if let Some(use_) = uses.get_mut(name) {
                use_.stores_field = true;
            }
        }
    }
}

/// Records an object construction or whole-local copy that a split can rewrite.
fn collect_whole_assignment(
    target: &Expr,
    value: &Expr,
    uses: &mut BTreeMap<String, LocalUse>,
) -> bool {
    let Expr::Ident(name) = target else {
        return false;
    };
    if !uses.contains_key(name) {
        return false;
    }
    match value {
        Expr::Object(entries) => {
            let mentions_self = entries.iter().any(|(_, value)| mentions(value, name));
            let use_ = uses.get_mut(name).expect("checked above");
            if mentions_self
                && !entries
                    .iter()
                    .all(|(field, value)| reads_only_own_field(value, name, field))
            {
                use_.opaque = true;
            } else {
                use_.self_update |= mentions_self;
                use_.written
                    .push(entries.iter().map(|(key, _)| key.clone()).collect());
            }
            for (_, value) in entries {
                collect_expr(value, uses);
            }
            true
        }
        Expr::Ident(source) if uses.contains_key(source) => {
            uses.get_mut(name)
                .expect("checked above")
                .copies
                .insert(source.clone());
            true
        }
        _ => false,
    }
}

/// Whether every read of `name` in `expr` is `name.field` for this assignment's own `field`.
///
/// Such an update remains correct when object replacement becomes one lane write at a time:
/// each right-hand side reads only the lane it is about to replace. Cross-field reads, delayed
/// closure reads, and assignments remain conservative refusals.
fn reads_only_own_field(expr: &Expr, name: &str, field: &str) -> bool {
    match expr {
        Expr::Ident(candidate) => candidate != name,
        Expr::Member(object, key) if matches!(object.as_ref(), Expr::Ident(candidate) if candidate == name) => {
            key == field
        }
        Expr::Member(object, _) | Expr::Unary(_, object) => {
            reads_only_own_field(object, name, field)
        }
        Expr::Index(object, index) | Expr::Binary(_, object, index) => {
            reads_only_own_field(object, name, field) && reads_only_own_field(index, name, field)
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            reads_only_own_field(callee, name, field)
                && args
                    .iter()
                    .all(|arg| reads_only_own_field(arg, name, field))
        }
        Expr::Cond(test, consequent, alternate) => {
            reads_only_own_field(test, name, field)
                && reads_only_own_field(consequent, name, field)
                && reads_only_own_field(alternate, name, field)
        }
        Expr::Object(entries) => entries
            .iter()
            .all(|(_, value)| reads_only_own_field(value, name, field)),
        Expr::Array(items) | Expr::Seq(items) => items
            .iter()
            .all(|item| reads_only_own_field(item, name, field)),
        Expr::Assign(..) | Expr::CompoundAssign(..) | Expr::Arrow(..) | Expr::Raw(_) => {
            !mentions(expr, name)
        }
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
    }
}

/// The two local operands of a runtime pointer equality call.
fn pointer_eq_args<'a>(callee: &'a Expr, args: &'a [Expr]) -> Option<(&'a str, &'a str)> {
    let Expr::Member(object, method) = callee else {
        return None;
    };
    if method != "ptr_eq" || !matches!(object.as_ref(), Expr::Ident(runtime) if runtime == "__rt") {
        return None;
    }
    let [Expr::Ident(left), Expr::Ident(right)] = args else {
        return None;
    };
    Some((left, right))
}

/// Whether `expr` mentions `name` at all.
fn mentions(expr: &Expr, name: &str) -> bool {
    let mut uses = BTreeMap::from([(name.to_owned(), LocalUse::default())]);
    collect_expr(expr, &mut uses);
    let use_ = &uses[name];
    use_.opaque || !use_.fields.is_empty()
}

type Split = BTreeMap<String, Vec<(String, String)>>;

fn apply_stmt(stmt: Stmt, split: &Split) -> Stmt {
    match stmt {
        Stmt::ExprStmt(Expr::Assign(target, value)) => {
            if let Some(writes) = split_assignment(&target, &value, split) {
                return Stmt::ExprStmt(writes);
            }
            Stmt::ExprStmt(Expr::Assign(
                Box::new(apply_expr(*target, split)),
                Box::new(apply_expr(*value, split)),
            ))
        }
        Stmt::ExprStmt(expr) => Stmt::ExprStmt(apply_expr(expr, split)),
        Stmt::Throw(expr) => Stmt::Throw(apply_expr(expr, split)),
        Stmt::Let(name, init) => match split.get(&name) {
            Some(fields) => Stmt::Lets(
                fields
                    .iter()
                    .map(|(_, scalar)| (scalar.clone(), None))
                    .collect(),
            ),
            None => Stmt::Let(name, init.map(|init| apply_expr(init, split))),
        },
        Stmt::Lets(declarations) => Stmt::Lets(
            declarations
                .into_iter()
                .flat_map(|(name, init)| match split.get(&name) {
                    Some(fields) => fields
                        .iter()
                        .map(|(_, scalar)| (scalar.clone(), None))
                        .collect(),
                    None => vec![(name, init.map(|init| apply_expr(init, split)))],
                })
                .collect(),
        ),
        Stmt::Const(name, init) => Stmt::Const(name, apply_expr(init, split)),
        Stmt::Return(value) => Stmt::Return(value.map(|value| apply_expr(value, split))),
        Stmt::If(test, then, otherwise) => Stmt::If(
            apply_expr(test, split),
            apply_stmts(then, split),
            otherwise.map(|otherwise| apply_stmts(otherwise, split)),
        ),
        Stmt::Switch(disc, cases) => Stmt::Switch(
            apply_expr(disc, split),
            cases
                .into_iter()
                .map(|case| SwitchCase {
                    tests: case
                        .tests
                        .into_iter()
                        .map(|test| apply_expr(test, split))
                        .collect(),
                    body: apply_stmts(case.body, split),
                })
                .collect(),
        ),
        Stmt::Labeled(label, body) => Stmt::Labeled(label, Box::new(apply_stmt(*body, split))),
        Stmt::LoopForever(label, body) => Stmt::LoopForever(label, apply_stmts(body, split)),
        Stmt::While(test, body) => Stmt::While(apply_expr(test, split), apply_stmts(body, split)),
        Stmt::FunctionDecl { name, params, body } => Stmt::FunctionDecl {
            name,
            params,
            body: apply_stmts(body, split),
        },
        Stmt::TryCatch { .. } => stmt,
        Stmt::Block(body) => Stmt::Block(apply_stmts(body, split)),
        Stmt::Continue(_)
        | Stmt::Break(_)
        | Stmt::Import { .. }
        | Stmt::Comment(_)
        | Stmt::Raw(_)
        | Stmt::Loc(_) => stmt,
    }
}

fn apply_stmts(stmts: Vec<Stmt>, split: &Split) -> Vec<Stmt> {
    stmts
        .into_iter()
        .map(|stmt| apply_stmt(stmt, split))
        .collect()
}

fn apply_expr(expr: Expr, split: &Split) -> Expr {
    match expr {
        Expr::Member(obj, key) => {
            if let Expr::Ident(name) = obj.as_ref() {
                if let Some(fields) = split.get(name) {
                    let scalar = fields
                        .iter()
                        .find(|(field, _)| *field == key)
                        .map(|(_, scalar)| scalar.clone())
                        .expect("every field read was included in the split shape");
                    return jsast::id(scalar);
                }
            }
            Expr::Member(Box::new(apply_expr(*obj, split)), key)
        }
        Expr::Index(obj, index) => Expr::Index(
            Box::new(apply_expr(*obj, split)),
            Box::new(apply_expr(*index, split)),
        ),
        Expr::Call(callee, args) => {
            if let Some((left, right)) = pointer_eq_args(&callee, &args) {
                if split.contains_key(left) && split.contains_key(right) {
                    let equal = ["buf", "off", "sc"]
                        .into_iter()
                        .map(|field| {
                            jsast::binary(
                                BinOp::StrictEq,
                                split_field_expr(left, field, split),
                                split_field_expr(right, field, split),
                            )
                        })
                        .reduce(|left, right| jsast::binary(BinOp::And, left, right))
                        .expect("pointer equality has fields");
                    return equal;
                }
            }
            Expr::Call(
                Box::new(apply_expr(*callee, split)),
                args.into_iter().map(|arg| apply_expr(arg, split)).collect(),
            )
        }
        Expr::New(callee, args) => Expr::New(
            Box::new(apply_expr(*callee, split)),
            args.into_iter().map(|arg| apply_expr(arg, split)).collect(),
        ),
        Expr::Unary(op, operand) => Expr::Unary(op, Box::new(apply_expr(*operand, split))),
        Expr::Binary(op, lhs, rhs) => Expr::Binary(
            op,
            Box::new(apply_expr(*lhs, split)),
            Box::new(apply_expr(*rhs, split)),
        ),
        Expr::Assign(target, value) => {
            if let Some(writes) = split_assignment(&target, &value, split) {
                return writes;
            }
            Expr::Assign(
                Box::new(apply_expr(*target, split)),
                Box::new(apply_expr(*value, split)),
            )
        }
        Expr::CompoundAssign(op, lhs, rhs) => Expr::CompoundAssign(
            op,
            Box::new(apply_expr(*lhs, split)),
            Box::new(apply_expr(*rhs, split)),
        ),
        Expr::Cond(test, consequent, alternate) => Expr::Cond(
            Box::new(apply_expr(*test, split)),
            Box::new(apply_expr(*consequent, split)),
            Box::new(apply_expr(*alternate, split)),
        ),
        Expr::Object(entries) => Expr::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, apply_expr(value, split)))
                .collect(),
        ),
        Expr::Array(items) => Expr::Array(
            items
                .into_iter()
                .map(|item| apply_expr(item, split))
                .collect(),
        ),
        Expr::Arrow(params, ArrowBody::Expr(body)) => {
            Expr::Arrow(params, ArrowBody::Expr(Box::new(apply_expr(*body, split))))
        }
        Expr::Arrow(params, ArrowBody::Block(body)) => {
            Expr::Arrow(params, ArrowBody::Block(apply_stmts(body, split)))
        }
        Expr::Seq(items) => Expr::Seq(
            items
                .into_iter()
                .map(|item| apply_expr(item, split))
                .collect(),
        ),
        other => other,
    }
}

/// One field lane of a split local.
fn split_field_expr(name: &str, field: &str, split: &Split) -> Expr {
    split
        .get(name)
        .and_then(|fields| fields.iter().find(|(candidate, _)| candidate == field))
        .map(|(_, scalar)| jsast::id(scalar.clone()))
        .unwrap_or_else(jsast::undefined)
}

/// Rewrites `local = { ... }` and `local = other` as lane assignments.
fn split_assignment(target: &Expr, value: &Expr, split: &Split) -> Option<Expr> {
    let Expr::Ident(name) = target else {
        return None;
    };
    let fields = split.get(name)?;
    let mut writes = Vec::new();
    match value {
        Expr::Object(entries) => {
            for (key, value) in entries {
                let scalar = fields
                    .iter()
                    .find(|(field, _)| field == key)
                    .map(|(_, scalar)| scalar)
                    .expect("every written field was included in the split shape");
                writes.push(jsast::assign(
                    jsast::id(scalar.clone()),
                    apply_expr(value.clone(), split),
                ));
            }
            for (field, scalar) in fields {
                if !entries.iter().any(|(key, _)| key == field) {
                    writes.push(jsast::assign(jsast::id(scalar.clone()), jsast::undefined()));
                }
            }
        }
        Expr::Ident(source) => {
            for (field, scalar) in fields {
                let value = if split.contains_key(source) {
                    split_field_expr(source, field, split)
                } else {
                    jsast::member(jsast::id(source.clone()), field.clone())
                };
                writes.push(jsast::assign(jsast::id(scalar.clone()), value));
            }
        }
        _ => return None,
    }
    Some(Expr::Seq(writes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(expr: Expr) -> Expr {
        rewrite_expr(expr)
    }

    fn bytes_to_int(operand: Expr) -> Expr {
        let read = |index| jsast::index(jsast::id("$b"), jsast::num(f64::from(index)));
        let body = jsast::binary(
            BinOp::BitOr,
            read(0),
            jsast::binary(BinOp::Shl, read(1), jsast::num(8.0)),
        );
        jsast::call(jsast::arrow(vec!["$b".to_owned()], body), vec![operand])
    }

    #[test]
    fn a_copy_of_a_fresh_literal_is_the_literal() {
        let literal = jsast::array(vec![jsast::id("a"), jsast::id("b")]);
        let copy = jsast::method_call(literal.clone(), "slice", vec![]);
        assert_eq!(rewrite(copy), literal);
    }

    #[test]
    fn a_copy_of_a_named_array_is_kept() {
        let copy = jsast::method_call(jsast::id("xs"), "slice", vec![]);
        assert_eq!(rewrite(copy.clone()), copy);
    }

    #[test]
    fn chained_copies_of_a_fresh_literal_collapse() {
        let literal = jsast::array(vec![jsast::id("a")]);
        let copy = jsast::method_call(
            jsast::method_call(literal.clone(), "slice", vec![]),
            "slice",
            vec![],
        );
        assert_eq!(rewrite(copy), literal);
    }

    #[test]
    fn a_literal_operand_becomes_the_elements() {
        let call = bytes_to_int(jsast::array(vec![jsast::id("x"), jsast::id("y")]));
        assert_eq!(
            rewrite(call).to_string(),
            jsast::binary(
                BinOp::BitOr,
                jsast::id("x"),
                jsast::binary(BinOp::Shl, jsast::id("y"), jsast::num(8.0)),
            )
            .to_string()
        );
    }

    #[test]
    fn an_effectful_element_keeps_its_position() {
        let call = bytes_to_int(jsast::array(vec![
            jsast::call(jsast::id("f"), vec![]),
            jsast::call(jsast::id("g"), vec![]),
        ]));
        assert_eq!(rewrite(call).to_string(), "f() | g() << 8");
    }

    #[test]
    fn a_copied_literal_operand_is_forwarded_too() {
        let call = bytes_to_int(jsast::method_call(
            jsast::array(vec![jsast::id("x"), jsast::id("y")]),
            "slice",
            vec![],
        ));
        assert_eq!(rewrite(call).to_string(), "x | y << 8");
    }

    #[test]
    fn a_named_operand_keeps_its_binding() {
        let call = bytes_to_int(jsast::id("bytes"));
        assert_eq!(rewrite(call.clone()), call);
    }

    #[test]
    fn a_wrong_length_operand_is_declined() {
        let call = bytes_to_int(jsast::array(vec![jsast::id("x")]));
        assert_eq!(rewrite(call.clone()), call);
    }

    #[test]
    fn an_out_of_order_read_is_declined() {
        let read = |index| jsast::index(jsast::id("$b"), jsast::num(f64::from(index)));
        let body = jsast::binary(BinOp::BitOr, read(1), read(0));
        let call = jsast::call(
            jsast::arrow(vec!["$b".to_owned()], body),
            vec![jsast::array(vec![jsast::id("x"), jsast::id("y")])],
        );
        assert_eq!(rewrite(call.clone()), call);
    }

    #[test]
    fn a_repeated_read_is_declined() {
        let read = |index| jsast::index(jsast::id("$b"), jsast::num(f64::from(index)));
        let body = jsast::binary(
            BinOp::BitOr,
            read(0),
            jsast::binary(BinOp::BitOr, read(0), read(1)),
        );
        let call = jsast::call(
            jsast::arrow(vec!["$b".to_owned()], body),
            vec![jsast::array(vec![jsast::id("x"), jsast::id("y")])],
        );
        assert_eq!(rewrite(call.clone()), call);
    }

    #[test]
    fn a_whole_parameter_use_is_declined() {
        let call = jsast::call(
            jsast::arrow(vec!["$b".to_owned()], jsast::id("$b")),
            vec![jsast::array(vec![jsast::id("x")])],
        );
        assert_eq!(rewrite(call.clone()), call);
    }

    #[test]
    fn a_conditional_body_is_declined() {
        let read = |index| jsast::index(jsast::id("$b"), jsast::num(f64::from(index)));
        let body = jsast::cond(jsast::id("t"), read(0), read(1));
        let call = jsast::call(
            jsast::arrow(vec!["$b".to_owned()], body),
            vec![jsast::array(vec![jsast::id("x"), jsast::id("y")])],
        );
        assert_eq!(rewrite(call.clone()), call);
    }

    #[test]
    fn rewrites_reach_into_statements() {
        let literal = jsast::array(vec![jsast::id("a")]);
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: Vec::new(),
            body: vec![Stmt::Return(Some(jsast::method_call(
                literal.clone(),
                "slice",
                vec![],
            )))],
        };
        let Stmt::FunctionDecl { body, .. } = rewrite_stmt(decl) else {
            panic!("expected a function declaration")
        };
        assert_eq!(body, vec![Stmt::Return(Some(literal))]);
    }

    /// `function f(p) { let <locals>; <body> }` after splitting, printed.
    fn split(locals: &[&str], params: &[&str], body: Vec<Stmt>) -> String {
        let mut statements = vec![Stmt::Lets(
            locals
                .iter()
                .map(|name| ((*name).to_owned(), None))
                .collect(),
        )];
        statements.extend(body);
        split_locals(
            Stmt::FunctionDecl {
                name: "f".to_owned(),
                params: params.iter().map(|name| (*name).to_owned()).collect(),
                body: statements,
            },
            true,
        )
        .to_string()
    }

    fn pair(target: &str, high: Expr, low: Expr) -> Stmt {
        jsast::assign_stmt(
            jsast::id(target),
            jsast::object(vec![("high".to_owned(), high), ("low".to_owned(), low)]),
        )
    }

    #[test]
    fn a_field_only_local_becomes_one_local_per_field() {
        let printed = split(
            &["v"],
            &[],
            vec![
                pair("v", jsast::id("a"), jsast::id("b")),
                Stmt::Return(Some(jsast::binary(
                    BinOp::BitOr,
                    jsast::member(jsast::id("v"), "high"),
                    jsast::member(jsast::id("v"), "low"),
                ))),
            ],
        );
        assert!(printed.contains("let v$high, v$low;"), "{printed}");
        assert!(printed.contains("v$high = a, v$low = b;"), "{printed}");
        assert!(printed.contains("return v$high | v$low;"), "{printed}");
    }

    #[test]
    fn a_whole_assignment_inside_a_sequence_can_be_split() {
        let printed = split(
            &["v"],
            &[],
            vec![
                Stmt::ExprStmt(Expr::Seq(vec![jsast::assign(
                    jsast::id("v"),
                    jsast::object(vec![
                        ("high".to_owned(), jsast::id("a")),
                        ("low".to_owned(), jsast::id("b")),
                    ]),
                )])),
                Stmt::Return(Some(jsast::member(jsast::id("v"), "high"))),
            ],
        );
        assert!(printed.contains("let v$high, v$low;"), "{printed}");
        assert!(printed.contains("v$high = a, v$low = b"), "{printed}");
        assert!(printed.contains("return v$high;"), "{printed}");
    }

    #[test]
    fn a_stable_object_alias_exposes_the_root_fields() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: Vec::new(),
            body: vec![
                Stmt::Lets(vec![("v".to_owned(), None), ("alias".to_owned(), None)]),
                pair("v", jsast::id("a"), jsast::id("b")),
                jsast::assign_stmt(jsast::id("alias"), jsast::id("v")),
                jsast::assign_stmt(jsast::member(jsast::id("alias"), "low"), jsast::id("c")),
                Stmt::Return(Some(jsast::member(jsast::id("alias"), "high"))),
            ],
        };
        let printed = split_locals(forward_stable_aliases(decl), true).to_string();
        assert!(!printed.contains("alias = v"), "{printed}");
        assert!(printed.contains("v$low = c;"), "{printed}");
        assert!(printed.contains("return v$high;"), "{printed}");
    }

    #[test]
    fn stable_scalar_alias_chains_are_forwarded() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["input".to_owned()],
            body: vec![
                Stmt::Lets(vec![
                    ("first".to_owned(), None),
                    ("second".to_owned(), None),
                    ("third".to_owned(), None),
                ]),
                jsast::assign_stmt(jsast::id("first"), jsast::id("input")),
                jsast::assign_stmt(jsast::id("second"), jsast::id("first")),
                jsast::assign_stmt(jsast::id("third"), jsast::id("second")),
                Stmt::Return(Some(jsast::id("third"))),
            ],
        };

        let printed = forward_stable_aliases(decl).to_string();

        assert!(!printed.contains("first = input"), "{printed}");
        assert!(!printed.contains("second = first"), "{printed}");
        assert!(!printed.contains("third = second"), "{printed}");
        assert!(printed.contains("return input;"), "{printed}");
    }

    #[test]
    fn an_alias_of_a_reassigned_value_keeps_its_snapshot() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: Vec::new(),
            body: vec![
                Stmt::Lets(vec![("value".to_owned(), None), ("alias".to_owned(), None)]),
                jsast::assign_stmt(jsast::id("value"), jsast::num(1.0)),
                jsast::assign_stmt(jsast::id("alias"), jsast::id("value")),
                jsast::assign_stmt(jsast::id("value"), jsast::num(2.0)),
                Stmt::Return(Some(jsast::id("alias"))),
            ],
        };

        let printed = forward_stable_aliases(decl).to_string();

        assert!(printed.contains("alias = value;"), "{printed}");
        assert!(printed.contains("return alias;"), "{printed}");
    }

    #[test]
    fn a_single_anchor_copy_cycle_is_forwarded() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["input".to_owned()],
            body: vec![
                Stmt::Lets(vec![("cursor".to_owned(), None), ("copy".to_owned(), None)]),
                jsast::assign_stmt(jsast::id("cursor"), jsast::id("input")),
                Stmt::LoopForever(
                    None,
                    vec![
                        jsast::assign_stmt(jsast::id("copy"), jsast::id("cursor")),
                        jsast::assign_stmt(jsast::id("cursor"), jsast::id("copy")),
                        jsast::assign_stmt(jsast::id("copy"), jsast::id("cursor")),
                        jsast::assign_stmt(jsast::id("cursor"), jsast::id("copy")),
                        jsast::assign_stmt(jsast::id("copy"), jsast::id("cursor")),
                        jsast::assign_stmt(jsast::id("cursor"), jsast::id("copy")),
                        jsast::assign_stmt(jsast::id("copy"), jsast::id("cursor")),
                        jsast::assign_stmt(jsast::id("cursor"), jsast::id("copy")),
                        Stmt::Break(None),
                    ],
                ),
                Stmt::Return(Some(jsast::id("copy"))),
            ],
        };

        let printed = forward_stable_aliases(decl).to_string();

        assert!(!printed.contains("cursor ="), "{printed}");
        assert!(!printed.contains("copy ="), "{printed}");
        assert!(printed.contains("return input;"), "{printed}");
    }

    #[test]
    fn a_small_copy_cycle_is_below_the_profitability_floor() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["input".to_owned()],
            body: vec![
                Stmt::Lets(vec![("cursor".to_owned(), None), ("copy".to_owned(), None)]),
                jsast::assign_stmt(jsast::id("cursor"), jsast::id("input")),
                Stmt::LoopForever(
                    None,
                    vec![
                        jsast::assign_stmt(jsast::id("copy"), jsast::id("cursor")),
                        jsast::assign_stmt(jsast::id("cursor"), jsast::id("copy")),
                        Stmt::Break(None),
                    ],
                ),
                Stmt::Return(Some(jsast::id("copy"))),
            ],
        };

        let printed = forward_stable_aliases(decl).to_string();

        assert!(printed.contains("cursor = input;"), "{printed}");
        assert!(printed.contains("copy = cursor;"), "{printed}");
    }

    #[test]
    fn a_copy_cycle_does_not_use_a_property_snapshot_as_its_anchor() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["input".to_owned()],
            body: vec![
                Stmt::Lets(vec![
                    ("anchor".to_owned(), None),
                    ("cursor".to_owned(), None),
                    ("copy".to_owned(), None),
                ]),
                jsast::assign_stmt(
                    jsast::id("anchor"),
                    jsast::member(jsast::id("input"), "field"),
                ),
                jsast::assign_stmt(jsast::id("cursor"), jsast::id("anchor")),
                jsast::assign_stmt(jsast::id("copy"), jsast::id("cursor")),
                jsast::assign_stmt(jsast::id("cursor"), jsast::id("copy")),
                Stmt::Return(Some(jsast::id("copy"))),
            ],
        };

        let printed = forward_stable_aliases(decl).to_string();

        assert!(printed.contains("anchor = input.field;"), "{printed}");
        assert!(printed.contains("cursor = anchor;"), "{printed}");
        assert!(printed.contains("copy = cursor;"), "{printed}");
    }

    #[test]
    fn a_copy_component_with_two_anchors_is_kept() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["left".to_owned(), "right".to_owned()],
            body: vec![
                Stmt::Lets(vec![("value".to_owned(), None), ("copy".to_owned(), None)]),
                jsast::assign_stmt(jsast::id("value"), jsast::id("left")),
                jsast::assign_stmt(jsast::id("copy"), jsast::id("value")),
                jsast::assign_stmt(jsast::id("value"), jsast::id("right")),
                Stmt::Return(Some(jsast::id("copy"))),
            ],
        };

        let printed = forward_stable_aliases(decl).to_string();

        assert!(printed.contains("copy = value;"), "{printed}");
        assert!(printed.contains("return copy;"), "{printed}");
    }

    #[test]
    fn a_copy_component_with_a_non_copy_write_is_kept() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["input".to_owned()],
            body: vec![
                Stmt::Lets(vec![("value".to_owned(), None), ("copy".to_owned(), None)]),
                jsast::assign_stmt(jsast::id("value"), jsast::id("input")),
                jsast::assign_stmt(jsast::id("copy"), jsast::id("value")),
                jsast::assign_stmt(jsast::id("value"), jsast::num(1.0)),
                Stmt::Return(Some(jsast::id("copy"))),
            ],
        };

        let printed = forward_stable_aliases(decl).to_string();

        assert!(printed.contains("copy = value;"), "{printed}");
        assert!(printed.contains("return copy;"), "{printed}");
    }

    #[test]
    fn field_snapshots_survive_mutation_through_an_alias() {
        let mutations = [
            jsast::call(jsast::id("mutate"), vec![jsast::id("alias")]),
            jsast::assign(
                Expr::Index(
                    Box::new(jsast::id("alias")),
                    Box::new(Expr::Str("high".to_owned())),
                ),
                jsast::num(7.0),
            ),
            jsast::call(
                jsast::id("mutate_nested"),
                vec![jsast::object(vec![(
                    "value".to_owned(),
                    jsast::id("alias"),
                )])],
            ),
        ];
        for mutation in mutations {
            let decl = Stmt::FunctionDecl {
                name: "f".to_owned(),
                params: Vec::new(),
                body: vec![
                    Stmt::Lets(vec![
                        ("v".to_owned(), None),
                        ("alias".to_owned(), None),
                        ("snapshot".to_owned(), None),
                    ]),
                    pair("v", jsast::num(1.0), jsast::num(2.0)),
                    jsast::assign_stmt(jsast::id("alias"), jsast::id("v")),
                    jsast::assign_stmt(
                        jsast::id("snapshot"),
                        jsast::member(jsast::id("alias"), "high"),
                    ),
                    Stmt::ExprStmt(mutation),
                    Stmt::Return(Some(jsast::id("snapshot"))),
                ],
            };
            let printed = forward_stable_aliases(decl).to_string();
            assert!(printed.contains("snapshot = v.high;"), "{printed}");
            assert!(printed.contains("return snapshot;"), "{printed}");
            assert!(!printed.contains("alias = v;"), "{printed}");
        }
    }

    #[test]
    fn only_private_own_field_snapshots_are_forwarded() {
        for nested in [false, true] {
            let field = jsast::member(jsast::id("v"), "high");
            let read = if nested {
                jsast::member(field, "off")
            } else {
                field
            };
            let decl = Stmt::FunctionDecl {
                name: "f".to_owned(),
                params: vec!["input".to_owned()],
                body: vec![
                    Stmt::Lets(vec![("v".to_owned(), None), ("snapshot".to_owned(), None)]),
                    pair("v", jsast::id("input"), jsast::num(2.0)),
                    jsast::assign_stmt(jsast::id("snapshot"), read),
                    Stmt::ExprStmt(jsast::call(jsast::id("mutate"), vec![jsast::id("input")])),
                    Stmt::Return(Some(jsast::id("snapshot"))),
                ],
            };
            let printed = forward_stable_aliases(decl).to_string();
            if nested {
                assert!(printed.contains("snapshot = v.high.off;"), "{printed}");
                assert!(printed.contains("return snapshot;"), "{printed}");
            } else {
                assert!(printed.contains("return v.high;"), "{printed}");
                assert!(!printed.contains("snapshot ="), "{printed}");
            }
        }
    }

    #[test]
    fn an_alias_of_a_replaced_field_keeps_its_snapshot() {
        let pointer =
            |offset| jsast::object(vec![("off".to_owned(), jsast::num(f64::from(offset)))]);
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: Vec::new(),
            body: vec![
                Stmt::Lets(vec![("v".to_owned(), None), ("alias".to_owned(), None)]),
                jsast::assign_stmt(
                    jsast::id("v"),
                    jsast::object(vec![("ptr".to_owned(), pointer(0))]),
                ),
                jsast::assign_stmt(jsast::id("alias"), jsast::member(jsast::id("v"), "ptr")),
                jsast::assign_stmt(jsast::member(jsast::id("v"), "ptr"), pointer(1)),
                Stmt::Return(Some(jsast::member(jsast::id("alias"), "off"))),
            ],
        };
        let printed = forward_stable_aliases(decl).to_string();
        assert!(printed.contains("alias = v.ptr;"), "{printed}");
        assert!(printed.contains("return alias.off;"), "{printed}");
    }

    #[test]
    fn a_field_write_is_split_too() {
        let printed = split(
            &["v"],
            &[],
            vec![
                pair("v", jsast::num(0.0), jsast::num(1.0)),
                jsast::assign_stmt(jsast::member(jsast::id("v"), "low"), jsast::num(2.0)),
            ],
        );
        assert!(printed.contains("v$low = 2;"), "{printed}");
    }

    #[test]
    fn a_local_used_whole_is_kept() {
        let printed = split(
            &["v"],
            &[],
            vec![
                pair("v", jsast::id("a"), jsast::id("b")),
                Stmt::ExprStmt(jsast::call(jsast::id("g"), vec![jsast::id("v")])),
            ],
        );
        assert!(printed.contains("v = { high: a, low: b };"), "{printed}");
    }

    #[test]
    fn variant_assignments_use_tag_and_payload_lanes() {
        let printed = split(
            &["v"],
            &[],
            vec![
                jsast::assign_stmt(
                    jsast::id("v"),
                    jsast::object(vec![
                        ("TAG".to_owned(), jsast::num(0.0)),
                        ("_0".to_owned(), jsast::id("a")),
                    ]),
                ),
                jsast::assign_stmt(
                    jsast::id("v"),
                    jsast::object(vec![
                        ("TAG".to_owned(), jsast::num(1.0)),
                        ("_1".to_owned(), jsast::id("c")),
                    ]),
                ),
                Stmt::Return(Some(jsast::member(jsast::id("v"), "_1"))),
            ],
        );
        assert!(!printed.contains("v = {"), "{printed}");
        assert!(
            printed.contains("v$TAG = 1, v$_1 = c, v$_0 = undefined;"),
            "{printed}"
        );
    }

    #[test]
    fn a_literal_that_reads_the_local_is_kept() {
        let printed = split(
            &["v"],
            &[],
            vec![
                pair("v", jsast::id("a"), jsast::id("b")),
                pair(
                    "v",
                    jsast::member(jsast::id("v"), "low"),
                    jsast::member(jsast::id("v"), "high"),
                ),
                Stmt::Return(Some(jsast::member(jsast::id("v"), "high"))),
            ],
        );
        assert!(
            printed.contains("v = { high: v.low, low: v.high };"),
            "{printed}"
        );
    }

    #[test]
    fn a_missing_payload_field_reads_as_undefined() {
        let printed = split(
            &["v"],
            &[],
            vec![
                jsast::assign_stmt(
                    jsast::id("v"),
                    jsast::object(vec![
                        ("TAG".to_owned(), jsast::num(0.0)),
                        ("_0".to_owned(), jsast::id("a")),
                    ]),
                ),
                Stmt::Return(Some(jsast::member(jsast::id("v"), "_1"))),
            ],
        );
        assert!(!printed.contains("v = {"), "{printed}");
        assert!(printed.contains("v$_1 = undefined;"), "{printed}");
    }

    #[test]
    fn copied_pointer_records_and_pointer_equality_stay_in_lanes() {
        let ptr = |name: &str, off: Expr| {
            jsast::assign_stmt(
                jsast::id(name),
                jsast::object(vec![
                    ("buf".to_owned(), jsast::id("bytes")),
                    ("off".to_owned(), off),
                ]),
            )
        };
        let printed = split(
            &["cursor", "snapshot", "end"],
            &[],
            vec![
                ptr("cursor", jsast::num(0.0)),
                ptr("end", jsast::num(4.0)),
                jsast::assign_stmt(jsast::id("snapshot"), jsast::id("cursor")),
                jsast::assign_stmt(
                    jsast::id("cursor"),
                    jsast::object(vec![
                        (
                            "buf".to_owned(),
                            jsast::member(jsast::id("snapshot"), "buf"),
                        ),
                        (
                            "off".to_owned(),
                            jsast::binary(
                                BinOp::Add,
                                jsast::member(jsast::id("snapshot"), "off"),
                                jsast::num(1.0),
                            ),
                        ),
                    ]),
                ),
                Stmt::Return(Some(jsast::rt_call(
                    "ptr_eq",
                    vec![jsast::id("cursor"), jsast::id("end")],
                ))),
            ],
        );
        assert!(!printed.contains("{ buf:"), "{printed}");
        assert!(!printed.contains("__rt.ptr_eq"), "{printed}");
        assert!(printed.contains("snapshot$buf = cursor$buf"), "{printed}");
        assert!(
            printed.contains("cursor$off = snapshot$off + 1"),
            "{printed}"
        );
    }

    #[test]
    fn a_field_local_pointer_update_stays_in_lanes() {
        let printed = split(
            &["cursor"],
            &[],
            vec![
                jsast::assign_stmt(
                    jsast::id("cursor"),
                    jsast::object(vec![
                        ("buf".to_owned(), jsast::id("bytes")),
                        ("off".to_owned(), jsast::num(0.0)),
                    ]),
                ),
                jsast::assign_stmt(
                    jsast::id("cursor"),
                    jsast::object(vec![
                        ("buf".to_owned(), jsast::member(jsast::id("cursor"), "buf")),
                        (
                            "off".to_owned(),
                            jsast::binary(
                                BinOp::Add,
                                jsast::member(jsast::id("cursor"), "off"),
                                jsast::num(1.0),
                            ),
                        ),
                    ]),
                ),
                Stmt::Return(Some(jsast::member(jsast::id("cursor"), "off"))),
            ],
        );
        assert!(!printed.contains("{ buf:"), "{printed}");
        assert!(printed.contains("cursor$off = cursor$off + 1"), "{printed}");
    }

    #[test]
    fn a_parameter_is_never_split() {
        let printed = split(
            &[],
            &["v"],
            vec![
                pair("v", jsast::id("a"), jsast::id("b")),
                Stmt::Return(Some(jsast::member(jsast::id("v"), "high"))),
            ],
        );
        assert!(printed.contains("v = { high: a, low: b };"), "{printed}");
    }

    #[test]
    fn verbatim_javascript_blocks_every_split() {
        let printed = split(
            &["v"],
            &[],
            vec![
                pair("v", jsast::id("a"), jsast::id("b")),
                Stmt::Raw("v.high = 1;".to_owned()),
                Stmt::Return(Some(jsast::member(jsast::id("v"), "high"))),
            ],
        );
        assert!(printed.contains("v = { high: a, low: b };"), "{printed}");
    }

    #[test]
    fn a_shadowing_arrow_parameter_blocks_the_split() {
        let printed = split(
            &["v"],
            &[],
            vec![
                pair("v", jsast::id("a"), jsast::id("b")),
                Stmt::Return(Some(jsast::arrow(
                    vec!["v".to_owned()],
                    jsast::member(jsast::id("v"), "high"),
                ))),
            ],
        );
        assert!(printed.contains("v = { high: a, low: b };"), "{printed}");
    }

    #[test]
    fn a_split_name_never_collides_with_an_existing_local() {
        let printed = split(
            &["v", "v$high"],
            &[],
            vec![
                pair("v", jsast::id("a"), jsast::id("b")),
                jsast::assign_stmt(jsast::id("v$high"), jsast::num(1.0)),
                Stmt::Return(Some(jsast::member(jsast::id("v"), "high"))),
            ],
        );
        assert!(printed.contains("v$high$ = a"), "{printed}");
        assert!(printed.contains("return v$high$;"), "{printed}");
    }

    /// `function f(c, x, y, v)` that builds `a` on two paths, names its object twice more, stores
    /// one field through the last name with `store`, and returns `a.f`.
    ///
    /// This is what an inlined `fn set(s: &mut S, v: u8) { s.f = v }` leaves behind: the
    /// callee's `&mut S` parameter is bound to the caller's object through `t = a; s = t`. The
    /// two paths keep `a` from being a once-written root whose aliases could be forwarded.
    fn store_through_alias(store: Expr) -> Stmt {
        let single = |value: &str| {
            jsast::assign_stmt(
                jsast::id("a"),
                jsast::object(vec![("f".to_owned(), jsast::id(value))]),
            )
        };
        Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: ["c", "x", "y", "v"].map(str::to_owned).to_vec(),
            body: vec![
                Stmt::Lets(["a", "t", "s"].map(|name| (name.to_owned(), None)).to_vec()),
                Stmt::If(jsast::id("c"), vec![single("x")], Some(vec![single("y")])),
                jsast::assign_stmt(jsast::id("t"), jsast::id("a")),
                jsast::assign_stmt(jsast::id("s"), jsast::id("t")),
                Stmt::ExprStmt(store),
                Stmt::Return(Some(jsast::member(jsast::id("a"), "f"))),
            ],
        }
    }

    #[test]
    fn a_store_through_a_second_name_reaches_the_object() {
        let field = || jsast::member(jsast::id("s"), "f");
        for (store, spelled) in [
            (jsast::assign(field(), jsast::id("v")), "s.f = v;"),
            (
                Expr::CompoundAssign(BinOp::Add, Box::new(field()), Box::new(jsast::id("v"))),
                "s.f += v;",
            ),
        ] {
            let decl = store_through_alias(store);
            let printed = split_locals(forward_stable_aliases(decl.clone()), true).to_string();
            assert!(printed.contains(spelled), "{printed}");
            assert!(printed.contains("return a.f;"), "{printed}");
            assert!(!printed.contains("a$f"), "{printed}");
            // The bounded JIR rounds pick among split candidates by size; none may lose the store.
            let printed = analyze_jir(decl, OptimizerMode::On).decl.to_string();
            assert!(printed.contains(spelled), "{printed}");
            assert!(printed.contains("return a.f;"), "{printed}");
        }
    }

    #[test]
    fn copies_without_a_store_are_still_split() {
        let read = jsast::assign(jsast::id("v"), jsast::member(jsast::id("s"), "f"));
        let printed =
            split_locals(forward_stable_aliases(store_through_alias(read)), true).to_string();
        assert!(printed.contains("t$f = a$f"), "{printed}");
        assert!(printed.contains("s$f = t$f"), "{printed}");
        assert!(printed.contains("return a$f;"), "{printed}");
    }

    #[test]
    fn a_store_into_a_fields_own_object_does_not_block_the_split() {
        // `s.f.g = v` changes the object both lanes `a$f` and `s$f` name, not a lane.
        let store = jsast::assign(
            jsast::member(jsast::member(jsast::id("s"), "f"), "g"),
            jsast::id("v"),
        );
        let printed =
            split_locals(forward_stable_aliases(store_through_alias(store)), true).to_string();
        assert!(printed.contains("s$f.g = v;"), "{printed}");
        assert!(printed.contains("return a$f;"), "{printed}");
    }

    #[test]
    fn a_store_without_a_second_name_is_split() {
        // Pointer equality connects two records without naming one object twice.
        let ptr = |name: &str, off: f64| {
            jsast::assign_stmt(
                jsast::id(name),
                jsast::object(vec![
                    ("buf".to_owned(), jsast::id("bytes")),
                    ("off".to_owned(), jsast::num(off)),
                ]),
            )
        };
        let printed = split(
            &["cursor", "end"],
            &[],
            vec![
                ptr("cursor", 0.0),
                ptr("end", 4.0),
                jsast::assign_stmt(jsast::member(jsast::id("cursor"), "off"), jsast::num(4.0)),
                Stmt::Return(Some(jsast::rt_call(
                    "ptr_eq",
                    vec![jsast::id("cursor"), jsast::id("end")],
                ))),
            ],
        );
        assert!(!printed.contains("{ buf:"), "{printed}");
        assert!(printed.contains("cursor$off = 4;"), "{printed}");
    }

    #[test]
    fn a_local_that_is_only_read_is_kept() {
        let printed = split(
            &["v"],
            &[],
            vec![Stmt::Return(Some(jsast::member(jsast::id("v"), "high")))],
        );
        assert!(printed.contains("return v.high;"), "{printed}");
    }
}
