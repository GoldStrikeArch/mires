//! Post-lowering slot deref and `__rt.sfp` folding passes:
//! - [`fold_offset_slot_derefs`]: eliminates single-assignment local `{ buf, off }` slot objects
//!   whose only reads are `v.buf[v.off]` within the same block.
//! - [`fold_sfp_deref_only`]: eliminates `__rt.sfp(ptr, len)` allocations on locals whose only
//!   observable reads are element dereferences `v.buf[v.off ...]` (and immediate `v.len` reads).

use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;

use rcgjs_object::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, Expr, Stmt, Visit, VisitMut,
};

use super::super::ast_util;

/// Shared write + closure-aware usage scanner parameterized by a [`LocalUsagePolicy`].
trait LocalUsagePolicy: Sized {
    type Usage: Default;

    fn record_write(usage: &mut Self::Usage);
    fn record_other_use(usage: &mut Self::Usage);
    fn try_match_expr(scanner: &mut LocalUsageScanner<'_, Self>, expr: &Expr) -> bool;
    fn try_match_assign_lhs(scanner: &mut LocalUsageScanner<'_, Self>, lhs: &Expr) -> bool;
}

struct LocalUsageScanner<'a, P: LocalUsagePolicy> {
    locals: &'a HashSet<String>,
    in_closure: bool,
    usage: HashMap<String, P::Usage>,
    _policy: PhantomData<P>,
}

impl<'a, P: LocalUsagePolicy> LocalUsageScanner<'a, P> {
    fn scan(stmts: &[Stmt], locals: &'a HashSet<String>) -> HashMap<String, P::Usage> {
        let mut scanner = Self {
            locals,
            in_closure: false,
            usage: HashMap::new(),
            _policy: PhantomData,
        };
        scanner.visit_stmts(stmts);
        scanner.usage
    }

    fn record_write(&mut self, name: &str) {
        P::record_write(self.usage.entry(name.to_owned()).or_default());
    }
}

impl<P: LocalUsagePolicy> Visit for LocalUsageScanner<'_, P> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                if !self.in_closure
                    && matches!(lhs.as_ref(), Expr::Ident(n) if self.locals.contains(n)) =>
            {
                let Expr::Ident(name) = lhs.as_ref() else {
                    unreachable!()
                };
                self.record_write(name);
                self.visit_expr(rhs);
            }
            Stmt::Let(name, Some(init)) if !self.in_closure && self.locals.contains(name) => {
                self.record_write(name);
                self.visit_expr(init);
            }
            Stmt::Lets(decls) => {
                for (name, init) in decls {
                    if let Some(init) = init {
                        if !self.in_closure && self.locals.contains(name) {
                            self.record_write(name);
                        }
                        self.visit_expr(init);
                    }
                }
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
        if !self.in_closure && P::try_match_expr(self, expr) {
            return;
        }
        match expr {
            Expr::Ident(name) if self.locals.contains(name) => {
                P::record_other_use(self.usage.entry(name.clone()).or_default());
            }
            Expr::Assign(lhs, rhs) => {
                if !self.in_closure && P::try_match_assign_lhs(self, lhs) {
                    self.visit_expr(rhs);
                    return;
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
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

fn is_v_member<'a>(expr: &'a Expr, prop: &str) -> Option<&'a str> {
    match expr {
        Expr::Member(obj, p) if p == prop => match obj.as_ref() {
            Expr::Ident(name) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// fold_offset_slot_derefs
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SlotUsage {
    writes: usize,
    slot_derefs: usize,
    other_uses: usize,
}

struct SlotUsagePolicy;

impl LocalUsagePolicy for SlotUsagePolicy {
    type Usage = SlotUsage;

    fn record_write(usage: &mut Self::Usage) {
        usage.writes += 1;
    }

    fn record_other_use(usage: &mut Self::Usage) {
        usage.other_uses += 1;
    }

    fn try_match_expr(scanner: &mut LocalUsageScanner<'_, Self>, expr: &Expr) -> bool {
        if let Some(name) = match_slot_deref(expr) {
            if scanner.locals.contains(name) {
                scanner
                    .usage
                    .entry(name.to_owned())
                    .or_default()
                    .slot_derefs += 1;
                return true;
            }
        }
        false
    }

    fn try_match_assign_lhs(scanner: &mut LocalUsageScanner<'_, Self>, lhs: &Expr) -> bool {
        Self::try_match_expr(scanner, lhs)
    }
}

#[derive(Clone)]
enum SimpleIdx {
    Ident(String),
    Num(f64),
    RawNum(String),
    Member(String, String),
}

impl SimpleIdx {
    fn to_expr(&self) -> Expr {
        match self {
            Self::Ident(s) => Expr::Ident(s.clone()),
            Self::Num(n) => Expr::Num(*n),
            Self::RawNum(s) => Expr::RawNum(s.clone()),
            Self::Member(obj, prop) => jsast::member(Expr::Ident(obj.clone()), prop.clone()),
        }
    }

    fn eq_idx(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ident(a), Self::Ident(b)) => a == b,
            (Self::Num(a), Self::Num(b)) => a.to_bits() == b.to_bits(),
            (Self::RawNum(a), Self::RawNum(b)) => a == b,
            (Self::Member(o1, p1), Self::Member(o2, p2)) => o1 == o2 && p1 == p2,
            _ => false,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum BaseRef {
    Ident(String),
    Member(String, String),
}

impl BaseRef {
    fn to_expr(&self) -> Expr {
        match self {
            Self::Ident(s) => Expr::Ident(s.clone()),
            Self::Member(obj, prop) => jsast::member(Expr::Ident(obj.clone()), prop.clone()),
        }
    }

    fn root_ident(&self) -> &str {
        match self {
            Self::Ident(s) | Self::Member(s, _) => s.as_str(),
        }
    }
}

#[derive(Clone)]
enum SlotDep {
    Ident(String),
    /// Member chain `obj.prop` (sensitive to `obj = ...`, `obj.prop = ...`, `obj.prop.{buf,off} = ...`,
    /// but NOT to sibling writes like `obj.pos = ...` or `obj.end = ...`).
    Member(String, String),
}

#[derive(Clone)]
enum SlotDef {
    Offset {
        base: BaseRef,
        op: jsast::BinOp,
        idx: SimpleIdx,
    },
    Direct {
        buf: BaseRef,
        off: SimpleIdx,
    },
}

impl SlotDef {
    fn replacement(&self) -> Expr {
        match self {
            Self::Offset { base, op, idx } => jsast::index(
                jsast::member(base.to_expr(), "buf"),
                jsast::binary(*op, jsast::member(base.to_expr(), "off"), idx.to_expr()),
            ),
            Self::Direct { buf, off } => jsast::index(buf.to_expr(), off.to_expr()),
        }
    }

    fn deps(&self) -> Vec<SlotDep> {
        let mut out = Vec::with_capacity(2);
        let push_base = |out: &mut Vec<SlotDep>, b: &BaseRef| match b {
            BaseRef::Ident(s) => out.push(SlotDep::Ident(s.clone())),
            BaseRef::Member(o, p) => out.push(SlotDep::Member(o.clone(), p.clone())),
        };
        let push_idx = |out: &mut Vec<SlotDep>, idx: &SimpleIdx| match idx {
            SimpleIdx::Ident(i) => out.push(SlotDep::Ident(i.clone())),
            SimpleIdx::Member(o, p) => out.push(SlotDep::Member(o.clone(), p.clone())),
            SimpleIdx::Num(_) | SimpleIdx::RawNum(_) => {}
        };
        match self {
            Self::Offset { base, idx, .. } => {
                push_base(&mut out, base);
                push_idx(&mut out, idx);
            }
            Self::Direct { buf, off } => {
                push_base(&mut out, buf);
                push_idx(&mut out, off);
            }
        }
        out
    }

    fn mentions_ident(&self, target: &str) -> bool {
        let idx_mentions = |idx: &SimpleIdx| match idx {
            SimpleIdx::Ident(s) | SimpleIdx::Member(s, _) => s == target,
            _ => false,
        };
        match self {
            Self::Offset { base, idx, .. } => base.root_ident() == target || idx_mentions(idx),
            Self::Direct { buf, off } => buf.root_ident() == target || idx_mentions(off),
        }
    }

    fn is_offset(&self) -> bool {
        matches!(self, Self::Offset { .. })
    }
}

fn match_base_ref(expr: &Expr) -> Option<BaseRef> {
    match expr {
        Expr::Ident(s) => Some(BaseRef::Ident(s.clone())),
        Expr::Member(obj, prop) => match obj.as_ref() {
            Expr::Ident(s) => Some(BaseRef::Member(s.clone(), prop.clone())),
            _ => None,
        },
        _ => None,
    }
}

fn match_simple_idx(expr: &Expr) -> Option<SimpleIdx> {
    match expr {
        Expr::Ident(s) => Some(SimpleIdx::Ident(s.clone())),
        Expr::Num(n) => Some(SimpleIdx::Num(*n)),
        Expr::RawNum(s) => Some(SimpleIdx::RawNum(s.clone())),
        Expr::Member(obj, prop) => match obj.as_ref() {
            Expr::Ident(s) => Some(SimpleIdx::Member(s.clone(), prop.clone())),
            _ => None,
        },
        _ => None,
    }
}

fn match_slot_deref(expr: &Expr) -> Option<&str> {
    let Expr::Index(obj, idx) = expr else {
        return None;
    };
    let b = is_v_member(obj, "buf")?;
    let o = is_v_member(idx, "off")?;
    (b == o).then_some(b)
}

fn match_offset_obj(expr: &Expr) -> Option<(BaseRef, jsast::BinOp, SimpleIdx)> {
    let Expr::Object(entries) = expr else {
        return None;
    };
    if entries.len() != 2 || entries[0].0 != "buf" || entries[1].0 != "off" {
        return None;
    }
    let Expr::Member(b1, bp) = &entries[0].1 else {
        return None;
    };
    if bp != "buf" {
        return None;
    }
    let base1 = match_base_ref(b1)?;
    let Expr::Binary(op @ (jsast::BinOp::Add | jsast::BinOp::Sub), lhs, rhs) = &entries[1].1 else {
        return None;
    };
    let Expr::Member(b2, op_prop) = lhs.as_ref() else {
        return None;
    };
    if op_prop != "off" {
        return None;
    }
    let base2 = match_base_ref(b2)?;
    if base1 != base2 {
        return None;
    }
    let idx = match_simple_idx(rhs)?;
    Some((base1, *op, idx))
}

fn match_slot_def(target: &str, rhs: &Expr) -> Option<SlotDef> {
    // Case 1: `idx === 0 ? base : { buf: base.buf, off: base.off +/- idx }`
    if let Expr::Cond(cond, then_e, else_e) = rhs {
        if let (
            Expr::Binary(jsast::BinOp::StrictEq, c_lhs, c_rhs),
            Some(then_base),
            Some((else_base, op, else_idx)),
        ) = (
            cond.as_ref(),
            match_base_ref(then_e),
            match_offset_obj(else_e),
        ) {
            if ast_util::is_zero_num(c_rhs)
                && then_base == else_base
                && match_simple_idx(c_lhs).is_some_and(|ci| ci.eq_idx(&else_idx))
            {
                let def = SlotDef::Offset {
                    base: else_base,
                    op,
                    idx: else_idx,
                };
                if !def.mentions_ident(target) {
                    return Some(def);
                }
            }
        }
        return None;
    }
    // Case 2: `{ buf: base.buf, off: base.off +/- idx }`
    if let Some((base, op, idx)) = match_offset_obj(rhs) {
        let def = SlotDef::Offset { base, op, idx };
        if !def.mentions_ident(target) {
            return Some(def);
        }
        return None;
    }
    // Case 3: `{ buf: arr, off: k }`
    if let Expr::Object(entries) = rhs {
        if entries.len() == 2 && entries[0].0 == "buf" && entries[1].0 == "off" {
            if let (Some(buf), Some(off)) = (
                match_base_ref(&entries[0].1),
                match_simple_idx(&entries[1].1),
            ) {
                let def = SlotDef::Direct { buf, off };
                if !def.mentions_ident(target) {
                    return Some(def);
                }
            }
        }
    }
    None
}

struct SlotDerefCounter<'a> {
    target: &'a str,
    count: usize,
}

impl Visit for SlotDerefCounter<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if match_slot_deref(expr) == Some(self.target) {
            self.count += 1;
            return;
        }
        if matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr(self, expr);
    }
}

fn count_derefs_in_stmt(stmt: &Stmt, target: &str) -> usize {
    let mut counter = SlotDerefCounter { target, count: 0 };
    counter.visit_stmt(stmt);
    counter.count
}

fn dep_matches_ident(deps: &[SlotDep], name: &str) -> bool {
    deps.iter().any(|d| match d {
        SlotDep::Ident(s) | SlotDep::Member(s, _) => s == name,
    })
}

fn lhs_writes_dep(lhs: &Expr, deps: &[SlotDep]) -> bool {
    match lhs {
        Expr::Ident(n) => dep_matches_ident(deps, n),
        Expr::Member(obj, prop) => match obj.as_ref() {
            Expr::Ident(n) => deps.iter().any(|d| match d {
                SlotDep::Ident(s) => s == n && (prop == "buf" || prop == "off"),
                SlotDep::Member(s, p) => s == n && p == prop,
            }),
            Expr::Member(inner, inner_prop) if prop == "buf" || prop == "off" => {
                matches!(inner.as_ref(), Expr::Ident(n) if deps.iter().any(|d| {
                    matches!(d, SlotDep::Member(s, p) if s == n && p == inner_prop)
                }))
            }
            _ => false,
        },
        _ => false,
    }
}

struct DepMutationChecker<'a> {
    deps: &'a [SlotDep],
    forbid_calls: bool,
    found: bool,
}

impl Visit for DepMutationChecker<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if self.found {
            return;
        }
        match stmt {
            Stmt::Let(n, _) | Stmt::Const(n, _) if dep_matches_ident(self.deps, n) => {
                self.found = true;
                return;
            }
            Stmt::Lets(decls) if decls.iter().any(|(n, _)| dep_matches_ident(self.deps, n)) => {
                self.found = true;
                return;
            }
            Stmt::FunctionDecl { .. } => return,
            _ => {}
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if self.found {
            return;
        }
        match expr {
            Expr::Assign(lhs, _) | Expr::CompoundAssign(_, lhs, _)
                if lhs_writes_dep(lhs, self.deps) =>
            {
                self.found = true;
            }
            Expr::Call(c, args) => {
                if ast_util::is_panic_callee(c) {
                    let prev = self.forbid_calls;
                    self.forbid_calls = false;
                    for a in args {
                        self.visit_expr(a);
                    }
                    self.forbid_calls = prev;
                    return;
                }
                if self.forbid_calls {
                    self.found = true;
                    return;
                }
                walk_expr(self, expr);
            }
            Expr::New(_, _) => {
                if self.forbid_calls {
                    self.found = true;
                    return;
                }
                walk_expr(self, expr);
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr(self, expr),
        }
    }
}

fn stmt_mutates_deps_or_calls(stmt: &Stmt, deps: &[SlotDep], forbid_calls: bool) -> bool {
    let mut checker = DepMutationChecker {
        deps,
        forbid_calls,
        found: false,
    };
    checker.visit_stmt(stmt);
    checker.found
}

struct SlotDerefReplacer<'a> {
    target: &'a str,
    replacement: &'a Expr,
}

impl VisitMut for SlotDerefReplacer<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if match_slot_deref(expr) == Some(self.target) {
            *expr = self.replacement.clone();
            return;
        }
        if matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr_mut(self, expr);
    }
}

fn rewrite_offset_slot_block(stmts: &mut Vec<Stmt>, eligible: &HashMap<String, usize>) {
    let mut i = 0;
    while i < stmts.len() {
        let matched = match &stmts[i] {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => match lhs.as_ref() {
                Expr::Ident(name) => eligible.get(name).and_then(|&total| {
                    match_slot_def(name, rhs).map(|def| (name.clone(), total, def))
                }),
                _ => None,
            },
            Stmt::Let(name, Some(rhs)) => eligible
                .get(name)
                .and_then(|&total| match_slot_def(name, rhs).map(|def| (name.clone(), total, def))),
            _ => None,
        };
        let Some((name, total_derefs, def)) = matched else {
            i += 1;
            continue;
        };
        let mut last_use = None;
        let mut local_derefs = 0usize;
        for (j, s) in stmts.iter().enumerate().skip(i + 1) {
            let c = count_derefs_in_stmt(s, &name);
            if c > 0 {
                local_derefs += c;
                last_use = Some(j);
            }
        }
        let Some(last) = last_use else {
            i += 1;
            continue;
        };
        if local_derefs != total_derefs {
            i += 1;
            continue;
        }
        let deps = def.deps();
        let forbid_calls = def.is_offset();
        let safe =
            (i + 1..=last).all(|j| !stmt_mutates_deps_or_calls(&stmts[j], &deps, forbid_calls));
        if !safe {
            i += 1;
            continue;
        }
        let replacement = def.replacement();
        let mut replacer = SlotDerefReplacer {
            target: &name,
            replacement: &replacement,
        };
        for s in &mut stmts[i + 1..=last] {
            replacer.visit_stmt_mut(s);
        }
        stmts.remove(i);
    }
}

/// Folds single-assignment local slot objects (`v = idx === 0 ? base : { buf: base.buf, off:
/// base.off + idx }`, `v = { buf: base.buf, off: base.off + idx }`, or `v = { buf: arr, off: k }`)
/// directly into `base.buf[base.off + idx]` / `arr[k]` when every read of `v` in the function is
/// strictly `v.buf[v.off]` within the same block and no dependency is mutated before the last use.
///
/// Eliminates per-iteration `{ buf, off }` slot allocations in `slice::Iter`, `slice::IterMut`,
/// `vec::IntoIter`, and `slice::swap` even across `Invoke` block boundaries where `fuse.rs` keeps
/// the offset result materialized.
pub(in super::super) fn fold_offset_slot_derefs(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
    let usage = LocalUsageScanner::<SlotUsagePolicy>::scan(body, locals);
    let eligible: HashMap<String, usize> = usage
        .into_iter()
        .filter(|(_, u)| u.writes == 1 && u.slot_derefs >= 1 && u.other_uses == 0)
        .map(|(name, u)| (name, u.slot_derefs))
        .collect();
    if !eligible.is_empty() {
        ast_util::visit_local_blocks_mut(body, false, |stmts| {
            rewrite_offset_slot_block(stmts, &eligible);
        });
    }
}

// ---------------------------------------------------------------------------
// fold_sfp_deref_only
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SfpUsage {
    writes: usize,
    index_uses: usize,
    len_reads: usize,
    other_uses: usize,
}

struct SfpUsagePolicy;

impl LocalUsagePolicy for SfpUsagePolicy {
    type Usage = SfpUsage;

    fn record_write(usage: &mut Self::Usage) {
        usage.writes += 1;
    }

    fn record_other_use(usage: &mut Self::Usage) {
        usage.other_uses += 1;
    }

    fn try_match_expr(scanner: &mut LocalUsageScanner<'_, Self>, expr: &Expr) -> bool {
        if Self::try_match_assign_lhs(scanner, expr) {
            return true;
        }
        if let Some(name) = is_v_member(expr, "len") {
            if scanner.locals.contains(name) {
                scanner.usage.entry(name.to_owned()).or_default().len_reads += 1;
                return true;
            }
        }
        false
    }

    fn try_match_assign_lhs(scanner: &mut LocalUsageScanner<'_, Self>, lhs: &Expr) -> bool {
        if let Expr::Index(obj, idx) = lhs {
            if let Some(name) = is_v_member(obj, "buf") {
                if scanner.locals.contains(name) && is_v_off_index(idx, name) {
                    scanner.usage.entry(name.to_owned()).or_default().index_uses += 1;
                    if let Expr::Binary(_, _, r) = idx.as_ref() {
                        scanner.visit_expr(r);
                    }
                    return true;
                }
            }
        }
        false
    }
}

fn is_v_off_index(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Member(obj, p) if p == "off" => matches!(obj.as_ref(), Expr::Ident(n) if n == name),
        Expr::Binary(jsast::BinOp::Add | jsast::BinOp::Sub, l, r) => {
            is_v_off_index(l, name) && !ast_util::expr_mentions_ident(r, name)
        }
        _ => false,
    }
}

fn match_sfp_call(expr: &Expr) -> Option<(&Expr, &Expr)> {
    match ast_util::rt_call(expr) {
        Some(("sfp", [ptr, len])) => Some((ptr, len)),
        _ => None,
    }
}

fn is_pure_len_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) => true,
        Expr::Member(obj, _) => is_pure_len_expr(obj),
        _ => false,
    }
}

fn collect_member_root_idents(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Ident(n) => {
            out.insert(n.clone());
        }
        Expr::Member(obj, _) => collect_member_root_idents(obj, out),
        _ => {}
    }
}

struct SfpDerefRewriter<'a>(&'a HashSet<String>);

impl VisitMut for SfpDerefRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if self.0.contains(n)) => {
                if let Some((ptr_arg, len_arg)) = match_sfp_call(rhs) {
                    if jsast::is_droppable(len_arg) || is_pure_len_expr(len_arg) {
                        **rhs = ptr_arg.clone();
                    }
                }
            }
            Stmt::Let(name, Some(init)) if self.0.contains(name) => {
                if let Some((ptr_arg, len_arg)) = match_sfp_call(init) {
                    if jsast::is_droppable(len_arg) || is_pure_len_expr(len_arg) {
                        *init = ptr_arg.clone();
                    }
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_mut(&mut self, _: &mut Expr) {}
}

struct VLenReplacer<'a> {
    var: &'a str,
    replacement: &'a Expr,
    count: usize,
}

impl VisitMut for VLenReplacer<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if is_v_member(expr, "len") == Some(self.var) {
            *expr = self.replacement.clone();
            self.count += 1;
            return;
        }
        if matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr_mut(self, expr);
    }
}

fn replace_v_len_in_expr(expr: &mut Expr, var: &str, replacement: &Expr) -> usize {
    let mut replacer = VLenReplacer {
        var,
        replacement,
        count: 0,
    };
    replacer.visit_expr_mut(expr);
    replacer.count
}

type ActiveSfpLens = HashMap<String, (Expr, HashSet<String>)>;

/// Replaces `v.len` reads in `stmts` with `len_expr` when `v = __rt.sfp(ptr, len_expr)` is
/// defined immediately before (only pure, non-aliasing local statements allowed in between)
/// and `len_expr` is a pure identifier/literal/member-chain whose root is unmodified.
struct SfpLenFolder<'a> {
    candidates: &'a HashSet<String>,
    folded_len_vars: HashSet<String>,
}

impl<'a> SfpLenFolder<'a> {
    fn new(candidates: &'a HashSet<String>) -> Self {
        Self {
            candidates,
            folded_len_vars: HashSet::new(),
        }
    }

    fn replace_active_lens(&mut self, expr: &mut Expr, active: &ActiveSfpLens) {
        for (var, (len_expr, _)) in active {
            if replace_v_len_in_expr(expr, var, len_expr) > 0 {
                self.folded_len_vars.insert(var.clone());
            }
        }
    }

    fn record_binding(&mut self, target: &str, rhs: &Expr, active: &mut ActiveSfpLens) {
        active.retain(|var, (_, deps)| var != target && !deps.contains(target));
        if self.candidates.contains(target) {
            if let Some((_, len_arg)) = match_sfp_call(rhs) {
                if is_pure_len_expr(len_arg) {
                    let mut deps = HashSet::new();
                    collect_member_root_idents(len_arg, &mut deps);
                    active.insert(target.to_owned(), (len_arg.clone(), deps));
                    return;
                }
            }
        }
        if !ast_util::is_side_effect_free_expr(rhs) {
            // A call or arbitrary effect may mutate `.len` fields on heap objects.
            active.retain(|_, (le, _)| !matches!(le, Expr::Member(_, _)));
        }
    }

    fn fold_expr(&mut self, expr: &mut Expr, active: &mut ActiveSfpLens) {
        // Handle comma-separated `Seq` of pure assignments sequentially.
        if let Expr::Seq(items) = expr {
            for item in items.iter_mut() {
                self.fold_expr(item, active);
            }
            return;
        }
        if let Expr::Assign(lhs, rhs) = expr {
            self.replace_active_lens(rhs, active);
            if let Expr::Ident(target) = lhs.as_ref() {
                self.record_binding(target, rhs, active);
                return;
            }
            // Property/index write: invalidate member-based lengths.
            active.retain(|_, (le, _)| !matches!(le, Expr::Member(_, _)));
            return;
        }
        self.replace_active_lens(expr, active);
        if !ast_util::is_side_effect_free_expr(expr) {
            active.clear();
        }
    }

    fn fold_stmts(&mut self, stmts: &mut [Stmt], active: &mut ActiveSfpLens) {
        for stmt in stmts.iter_mut() {
            match stmt {
                Stmt::ExprStmt(e) => self.fold_expr(e, active),
                Stmt::Let(target, Some(init)) => {
                    self.replace_active_lens(init, active);
                    self.record_binding(target, init, active);
                }
                Stmt::Block(b) => self.fold_stmts(b, active),
                Stmt::Labeled(_, inner) => {
                    let mut sub = HashMap::new();
                    self.fold_stmts(std::slice::from_mut(inner.as_mut()), &mut sub);
                    active.clear();
                }
                Stmt::If(cond, a, b) => {
                    self.replace_active_lens(cond, active);
                    let mut a_active = active.clone();
                    self.fold_stmts(a, &mut a_active);
                    if let Some(b) = b {
                        let mut b_active = active.clone();
                        self.fold_stmts(b, &mut b_active);
                    }
                    active.clear();
                }
                Stmt::Switch(disc, cases) => {
                    self.replace_active_lens(disc, active);
                    for c in cases {
                        let mut c_active = active.clone();
                        self.fold_stmts(&mut c.body, &mut c_active);
                    }
                    active.clear();
                }
                Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    let mut sub = HashMap::new();
                    self.fold_stmts(b, &mut sub);
                    active.clear();
                }
                Stmt::TryCatch { body, handler, .. } => {
                    let mut sub = HashMap::new();
                    self.fold_stmts(body, &mut sub);
                    let mut hsub = HashMap::new();
                    self.fold_stmts(handler, &mut hsub);
                    active.clear();
                }
                Stmt::Return(Some(e)) | Stmt::Throw(e) => {
                    self.replace_active_lens(e, active);
                    active.clear();
                }
                _ => active.clear(),
            }
        }
    }
}

fn count_ident_mentions(stmts: &[Stmt], target: &str) -> usize {
    let mut n = 0usize;
    for stmt in stmts {
        jsast::visit_idents(stmt, &mut |id| {
            if id == target {
                n += 1;
            }
        });
    }
    n
}

fn propagate_sfp_aliases_in_block(
    stmts: &mut Vec<Stmt>,
    sfp_vars: &HashSet<String>,
    single_write_aliases: &HashSet<String>,
    total_mentions: &HashMap<String, usize>,
) {
    let mut active_sfp: HashSet<String> = HashSet::new();
    let mut i = 0;
    while i < stmts.len() {
        if let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = &stmts[i] {
            if let Expr::Ident(target) = lhs.as_ref() {
                if sfp_vars.contains(target) && match_sfp_call(rhs).is_some() {
                    active_sfp.insert(target.clone());
                    i += 1;
                    continue;
                }
                if let Expr::Ident(src) = rhs.as_ref() {
                    if single_write_aliases.contains(target) && active_sfp.contains(src) {
                        let after_mentions = count_ident_mentions(&stmts[i + 1..], target);
                        if total_mentions.get(target).copied() == Some(after_mentions + 1) {
                            let from = target.clone();
                            let to = src.clone();
                            ast_util::rename_ident_stmts(&mut stmts[i + 1..], &from, &to);
                            stmts.remove(i);
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }
}

/// Propagates single-assignment aliases of `__rt.sfp` locals (`_3 = $v12`) inside straight-line
/// blocks when `_3` is never mentioned outside statements strictly following `_3 = $v12` (so
/// it cannot carry across a loop back-edge) and neither `_3` nor `$v12` is reassigned or
/// captured by a closure. This exposes `slice::Iter` (`_3 = $v12; _3$2 = $v12.len; ... _3.buf[...]`)
/// directly to [`SfpLenFolder`] and [`SfpDerefRewriter`].
fn propagate_sfp_aliases(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
    let usage = LocalUsageScanner::<SfpUsagePolicy>::scan(body, locals);
    let sfp_vars: HashSet<String> = usage
        .iter()
        .filter(|(name, u)| u.writes == 1 && !ast_util::has_closure_use(body, name))
        .map(|(name, _)| name.clone())
        .collect();
    let single_write_aliases: HashSet<String> = usage
        .iter()
        .filter(|(name, u)| {
            u.writes == 1
                && (u.index_uses > 0 || u.len_reads > 0)
                && u.other_uses == 0
                && !ast_util::has_closure_use(body, name)
        })
        .map(|(name, _)| name.clone())
        .collect();
    if single_write_aliases.is_empty() || sfp_vars.is_empty() {
        return;
    }
    let mut total_mentions: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        jsast::visit_idents(stmt, &mut |id| {
            if single_write_aliases.contains(id) {
                *total_mentions.entry(id.to_owned()).or_default() += 1;
            }
        });
    }
    ast_util::visit_local_blocks_mut(body, true, |stmts| {
        propagate_sfp_aliases_in_block(stmts, &sfp_vars, &single_write_aliases, &total_mentions);
    });
}

/// Eliminates `__rt.sfp(ptr, len)` allocations on single-assignment locals whose only observable
/// reads are element dereferences `v.buf[...]` (and optionally `v.len` within the same block while
/// `len` is unmodified).
///
/// When `v` never escapes as a slice record and is never inspected when `len == 0` (every element
/// access `v.buf[...]` requires `len > 0`, where `ptr` is already a valid `{ buf, off }` slot),
/// rewriting `v = __rt.sfp(ptr, len)` to `v = ptr` avoids both the `__rt.sfp` call and the
/// `{ buf, off, len }` allocation — including on empty slices (`len == 0`) where `ptr` is a
/// dangling integer that the loop never dereferences.
pub(in super::super) fn fold_sfp_deref_only(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
    propagate_sfp_aliases(body, locals);

    let mut usage = LocalUsageScanner::<SfpUsagePolicy>::scan(body, locals);
    let len_candidates: HashSet<String> = usage
        .iter()
        .filter(|(_, u)| {
            u.writes == 1 && u.index_uses >= 1 && u.len_reads >= 1 && u.other_uses == 0
        })
        .map(|(name, _)| name.clone())
        .collect();
    if !len_candidates.is_empty() {
        let mut folder = SfpLenFolder::new(&len_candidates);
        let mut active = HashMap::new();
        folder.fold_stmts(body, &mut active);
        if !folder.folded_len_vars.is_empty() {
            usage = LocalUsageScanner::<SfpUsagePolicy>::scan(body, locals);
        }
    }
    let deref_only: HashSet<String> = usage
        .into_iter()
        .filter(|(_, u)| {
            u.writes == 1 && u.index_uses >= 1 && u.len_reads == 0 && u.other_uses == 0
        })
        .map(|(name, _)| name)
        .collect();
    if !deref_only.is_empty() {
        SfpDerefRewriter(&deref_only).visit_stmts_mut(body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_offset_slot_derefs_folds_single_use_and_guards_mutations() {
        let mk_slot = |base: &str, idx: &str| {
            Expr::Cond(
                Box::new(Expr::Binary(
                    jsast::BinOp::StrictEq,
                    Box::new(jsast::id(idx)),
                    Box::new(jsast::num(0.0)),
                )),
                Box::new(jsast::id(base)),
                Box::new(Expr::Object(vec![
                    (
                        "buf".to_owned(),
                        Expr::Member(Box::new(jsast::id(base)), "buf".to_owned()),
                    ),
                    (
                        "off".to_owned(),
                        Expr::Binary(
                            jsast::BinOp::Add,
                            Box::new(Expr::Member(Box::new(jsast::id(base)), "off".to_owned())),
                            Box::new(jsast::id(idx)),
                        ),
                    ),
                ])),
            )
        };
        let slot_deref = |slot: &str| {
            Expr::Index(
                Box::new(Expr::Member(Box::new(jsast::id(slot)), "buf".to_owned())),
                Box::new(Expr::Member(Box::new(jsast::id(slot)), "off".to_owned())),
            )
        };
        let locals: HashSet<String> = ["slot".to_owned(), "out".to_owned()].into_iter().collect();

        // Positive: single-assignment offset slot folded into direct element read.
        let mut body = vec![
            jsast::assign_stmt(jsast::id("slot"), mk_slot("p", "i")),
            jsast::assign_stmt(jsast::id("out"), slot_deref("slot")),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_offset_slot_derefs(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(printed.contains("p.buf[p.off + i]"), "{printed}");
        assert!(!printed.contains("slot ="), "{printed}");

        // Negative: mutating `i` between def and deref must prevent folding.
        let mut mutated = vec![
            jsast::assign_stmt(jsast::id("slot"), mk_slot("p", "i")),
            jsast::assign_stmt(jsast::id("i"), jsast::num(9.0)),
            jsast::assign_stmt(jsast::id("out"), slot_deref("slot")),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_offset_slot_derefs(&mut mutated, &locals);
        let printed_mut = jsast::stmt_to_string(&Stmt::Block(mutated));
        assert!(printed_mut.contains("slot ="), "{printed_mut}");
        assert!(printed_mut.contains("slot.buf[slot.off]"), "{printed_mut}");
    }

    #[test]
    fn fold_sfp_deref_only_eliminates_sfp_for_deref_and_immediate_len() {
        let locals: HashSet<String> = ["s".to_owned(), "n".to_owned(), "out".to_owned()]
            .into_iter()
            .collect();
        let sfp_rhs = jsast::rt_call(
            "sfp",
            vec![
                Expr::Member(Box::new(jsast::id("v")), "ptr".to_owned()),
                Expr::Member(Box::new(jsast::id("v")), "len".to_owned()),
            ],
        );

        // Positive: `s = __rt.sfp(v.ptr, v.len); n = s.len; out = s.buf[s.off + i];`
        let mut body = vec![
            jsast::assign_stmt(jsast::id("s"), sfp_rhs.clone()),
            jsast::assign_stmt(
                jsast::id("n"),
                Expr::Member(Box::new(jsast::id("s")), "len".to_owned()),
            ),
            jsast::assign_stmt(
                jsast::id("out"),
                Expr::Index(
                    Box::new(Expr::Member(Box::new(jsast::id("s")), "buf".to_owned())),
                    Box::new(Expr::Binary(
                        jsast::BinOp::Add,
                        Box::new(Expr::Member(Box::new(jsast::id("s")), "off".to_owned())),
                        Box::new(jsast::id("i")),
                    )),
                ),
            ),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_sfp_deref_only(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("__rt.sfp"), "{printed}");
        assert!(printed.contains("s = v.ptr;"), "{printed}");
        assert!(printed.contains("n = v.len;"), "{printed}");

        // Negative: escaped `s` must keep `__rt.sfp`.
        let mut escaped = vec![
            jsast::assign_stmt(jsast::id("s"), sfp_rhs.clone()),
            Stmt::Return(Some(jsast::id("s"))),
        ];
        fold_sfp_deref_only(&mut escaped, &locals);
        let printed_esc = jsast::stmt_to_string(&Stmt::Block(escaped));
        assert!(printed_esc.contains("__rt.sfp"), "{printed_esc}");

        // Positive with single-assignment alias (`_3 = s; n = _3.len; out = _3.buf[_3.off + i]`).
        let locals_alias: HashSet<String> = [
            "s".to_owned(),
            "_3".to_owned(),
            "n".to_owned(),
            "out".to_owned(),
        ]
        .into_iter()
        .collect();
        let mut aliased = vec![
            jsast::assign_stmt(jsast::id("s"), sfp_rhs),
            jsast::assign_stmt(jsast::id("_3"), jsast::id("s")),
            jsast::assign_stmt(
                jsast::id("n"),
                Expr::Member(Box::new(jsast::id("_3")), "len".to_owned()),
            ),
            jsast::assign_stmt(
                jsast::id("out"),
                Expr::Index(
                    Box::new(Expr::Member(Box::new(jsast::id("_3")), "buf".to_owned())),
                    Box::new(Expr::Binary(
                        jsast::BinOp::Add,
                        Box::new(Expr::Member(Box::new(jsast::id("_3")), "off".to_owned())),
                        Box::new(jsast::id("i")),
                    )),
                ),
            ),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_sfp_deref_only(&mut aliased, &locals_alias);
        let printed_alias = jsast::stmt_to_string(&Stmt::Block(aliased));
        assert!(!printed_alias.contains("__rt.sfp"), "{printed_alias}");
        assert!(printed_alias.contains("n = v.len;"), "{printed_alias}");
    }
}
