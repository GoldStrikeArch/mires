//! Dominated redundant guard elimination and hot/cold loop-versioned BCE.

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp, Visit};

use super::super::ast_util::{
    collect_written_idents_expr, collect_written_idents_stmts_opt as collect_written_idents_stmts,
    exact_i32, exact_u32, exact_u64, expr_has_side_effect, stmts_definitely_diverge,
};
use super::unroll::{detect_loop_counter, CounterInfo};
use super::util::{
    count_stmts, expr_has_non_panic_call, expr_reads_ident, stmts_have_mutating_nested_fn,
};

// =================================================================================================
// 4. Dominated Redundant Guard Elimination & Constant-Table / Bitmask Bounds
// =================================================================================================

/// Pure expression equivalence key for dominated guard elimination (handles `g >>> 0`,
/// `g | 0`, `a & b`, `a + c`, `a - c`, and `obj.prop` with sound invalidation).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum PureKey {
    Const(u32),
    Ident(String),
    UShr0(Box<PureKey>),
    BitOr0(Box<PureKey>),
    BitAnd(Box<PureKey>, Box<PureKey>),
    AddConst(Box<PureKey>, u32),
    SubConst(Box<PureKey>, u32),
    Member(String, String),
}

impl PureKey {
    fn mentions_ident(&self, ident: &str) -> bool {
        match self {
            Self::Const(_) => false,
            Self::Ident(s) | Self::Member(s, _) => s == ident,
            Self::UShr0(a) | Self::BitOr0(a) | Self::AddConst(a, _) | Self::SubConst(a, _) => {
                a.mentions_ident(ident)
            }
            Self::BitAnd(a, b) => a.mentions_ident(ident) || b.mentions_ident(ident),
        }
    }

    fn has_member(&self) -> bool {
        match self {
            Self::Const(_) | Self::Ident(_) => false,
            Self::Member(_, _) => true,
            Self::UShr0(a) | Self::BitOr0(a) | Self::AddConst(a, _) | Self::SubConst(a, _) => {
                a.has_member()
            }
            Self::BitAnd(a, b) => a.has_member() || b.has_member(),
        }
    }
}

#[derive(Clone, Default)]
struct GuardEnv {
    known_true: HashSet<String>,
    known_false: HashSet<String>,
    /// Pairs `(idx_key, len_key)` proven `idx < len`.
    known_lt: HashSet<(PureKey, PureKey)>,
    /// Maps bool var `H` -> `(idx_key, len_key)` when `H = idx < len`.
    bool_lt_def: HashMap<String, (PureKey, PureKey)>,
    /// Maps local var `v` -> canonical pure expression (e.g. `t = g >>> 0` -> `UShr0(Ident("g"))`).
    expr_alias: HashMap<String, PureKey>,
    /// Maps local var `v` -> proven non-negative integer upper bound `<= u32`.
    upper_bounds: HashMap<String, u32>,
}

impl GuardEnv {
    fn invalidate(&mut self, ident: &str) {
        self.known_true.remove(ident);
        self.known_false.remove(ident);
        self.upper_bounds.remove(ident);
        self.expr_alias.remove(ident);
        self.expr_alias.retain(|_, k| !k.mentions_ident(ident));
        self.bool_lt_def.remove(ident);
        self.bool_lt_def
            .retain(|_, (a, b)| !a.mentions_ident(ident) && !b.mentions_ident(ident));
        self.known_lt
            .retain(|(a, b)| !a.mentions_ident(ident) && !b.mentions_ident(ident));
    }

    /// Invalidates all keys that read heap properties (`Member(obj, prop)`), preserving pure
    /// local scalar/cast equivalences like `t = g >>> 0; t < c` across slice/field mutations!
    fn invalidate_members(&mut self) {
        self.expr_alias.retain(|_, k| !k.has_member());
        self.bool_lt_def
            .retain(|_, (a, b)| !a.has_member() && !b.has_member());
        self.known_lt
            .retain(|(a, b)| !a.has_member() && !b.has_member());
    }

    fn resolve_key(&self, expr: &Expr) -> Option<PureKey> {
        self.resolve_key_depth(expr, 0)
    }

    fn resolve_key_depth(&self, expr: &Expr, depth: u8) -> Option<PureKey> {
        if depth >= 6 {
            return None;
        }
        if let Some(c) = exact_u32(expr) {
            return Some(PureKey::Const(c));
        }
        match expr {
            Expr::Ident(id) => Some(
                self.expr_alias
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| PureKey::Ident(id.clone())),
            ),
            Expr::Member(obj, prop) => match obj.as_ref() {
                Expr::Ident(base) => Some(PureKey::Member(base.clone(), prop.clone())),
                _ => None,
            },
            Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => {
                let inner = self.resolve_key_depth(l, depth + 1)?;
                // `UShr0(UShr0(x))` and `UShr0(Const(c))` are idempotent!
                match inner {
                    PureKey::Const(_) | PureKey::UShr0(_) => Some(inner),
                    other => Some(PureKey::UShr0(Box::new(other))),
                }
            }
            Expr::Binary(BinOp::BitOr, l, r) if exact_i32(r) == Some(0) => {
                let inner = self.resolve_key_depth(l, depth + 1)?;
                match inner {
                    PureKey::BitOr0(_) => Some(inner),
                    other => Some(PureKey::BitOr0(Box::new(other))),
                }
            }
            Expr::Binary(BinOp::BitAnd, l, r) => {
                let lk = self.resolve_key_depth(l, depth + 1)?;
                let rk = self.resolve_key_depth(r, depth + 1)?;
                Some(PureKey::BitAnd(Box::new(lk), Box::new(rk)))
            }
            Expr::Binary(BinOp::Add, l, r) => {
                if let Some(c) = exact_u32(r) {
                    let lk = self.resolve_key_depth(l, depth + 1)?;
                    Some(PureKey::AddConst(Box::new(lk), c))
                } else if let Some(c) = exact_u32(l) {
                    let rk = self.resolve_key_depth(r, depth + 1)?;
                    Some(PureKey::AddConst(Box::new(rk), c))
                } else {
                    None
                }
            }
            Expr::Binary(BinOp::Sub, l, r) => {
                let c = exact_u32(r)?;
                let lk = self.resolve_key_depth(l, depth + 1)?;
                Some(PureKey::SubConst(Box::new(lk), c))
            }
            _ => None,
        }
    }

    fn record_cond_true(&mut self, cond: &Expr) {
        match cond {
            Expr::Ident(h) => {
                self.known_true.insert(h.clone());
                if let Some((a, b)) = self.bool_lt_def.get(h).cloned() {
                    self.known_lt.insert((a.clone(), b.clone()));
                    if let (PureKey::Ident(var), PureKey::Const(c)) = (&a, &b) {
                        if *c >= 1 {
                            self.tighten_upper(var, c - 1);
                        }
                    }
                }
            }
            Expr::Unary(UnOp::Not, inner) => {
                if let Expr::Ident(b) = inner.as_ref() {
                    self.known_false.insert(b.clone());
                }
            }
            Expr::Binary(BinOp::Lt, l, r) => {
                if let (Some(lk), Some(rk)) = (self.resolve_key(l), self.resolve_key(r)) {
                    self.known_lt.insert((lk, rk));
                }
                if let (Expr::Ident(v), Some(c)) = (l.as_ref(), exact_u32(r)) {
                    if c >= 1 {
                        self.tighten_upper(v, c - 1);
                    }
                }
            }
            Expr::Binary(BinOp::Le, l, r) => {
                if let (Expr::Ident(v), Some(c)) = (l.as_ref(), exact_u32(r)) {
                    self.tighten_upper(v, c);
                }
            }
            Expr::Binary(BinOp::And, l, r) => {
                self.record_cond_true(l);
                self.record_cond_true(r);
            }
            _ => {}
        }
    }

    fn record_cond_false(&mut self, cond: &Expr) {
        if let Expr::Unary(UnOp::Not, inner) = cond {
            self.record_cond_true(inner);
        } else if let Expr::Ident(b) = cond {
            self.known_false.insert(b.clone());
        }
    }

    fn tighten_upper(&mut self, var: &str, bound: u32) {
        self.upper_bounds
            .entry(var.to_owned())
            .and_modify(|cur| *cur = (*cur).min(bound))
            .or_insert(bound);
    }

    fn expr_upper_bound(&self, expr: &Expr) -> Option<u32> {
        if let Some(c) = exact_u32(expr) {
            return Some(c);
        }
        match expr {
            Expr::Ident(v) => self.upper_bounds.get(v).copied(),
            Expr::Binary(BinOp::UShr, l, r) => {
                let shift = exact_u32(r)?;
                if shift == 0 {
                    self.expr_upper_bound(l)
                } else if shift < 32 {
                    let l_hi = self.expr_upper_bound(l).unwrap_or(u32::MAX);
                    Some(l_hi >> shift)
                } else {
                    None
                }
            }
            Expr::Binary(BinOp::BitAnd, l, r) => {
                let lh = self.expr_upper_bound(l);
                let rh = self.expr_upper_bound(r);
                match (lh, rh) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) if a <= i32::MAX as u32 => Some(a),
                    (None, Some(b)) if b <= i32::MAX as u32 => Some(b),
                    _ => None,
                }
            }
            Expr::Binary(BinOp::Add, l, r) => {
                let a = self.expr_upper_bound(l)?;
                let b = self.expr_upper_bound(r)?;
                a.checked_add(b)
            }
            Expr::Binary(BinOp::Rem, l, r) => {
                let m = exact_u32(r)?;
                if m == 0 {
                    return None;
                }
                let l_hi = self.expr_upper_bound(l)?;
                Some(l_hi.min(m - 1))
            }
            Expr::Index(arr, idx) => {
                // Constant scalar array literal `[c0, c1, ...][idx]` when `idx` is proven `< len`.
                let Expr::Array(items) = arr.as_ref() else {
                    return None;
                };
                if items.is_empty() {
                    return None;
                }
                let idx_hi = self.expr_upper_bound(idx)?;
                if (idx_hi as usize) >= items.len() {
                    return None;
                }
                let mut max_elem = 0u32;
                for item in items {
                    let v = exact_u32(item)?;
                    max_elem = max_elem.max(v);
                }
                Some(max_elem)
            }
            _ => None,
        }
    }

    fn guard_is_dead(&self, cond: &Expr) -> bool {
        match cond {
            Expr::Bool(false) => true,
            Expr::Ident(b) => self.known_false.contains(b),
            Expr::Unary(UnOp::Not, inner) => match inner.as_ref() {
                Expr::Ident(h) => {
                    self.known_true.contains(h)
                        || self
                            .bool_lt_def
                            .get(h)
                            .is_some_and(|pair| self.known_lt.contains(pair))
                }
                Expr::Binary(BinOp::Lt, l, r) => {
                    if let (Some(lk), Some(rk)) = (self.resolve_key(l), self.resolve_key(r)) {
                        if self.known_lt.contains(&(lk, rk)) {
                            return true;
                        }
                    }
                    if let Some(len_c) = exact_u32(r) {
                        if self.expr_upper_bound(l).is_some_and(|hi| hi < len_c) {
                            return true;
                        }
                    }
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }
}

pub(in super::super) fn eliminate_dominated_guards(stmts: &mut Vec<Stmt>) {
    if stmts_have_mutating_nested_fn(stmts) {
        return;
    }
    let mut env = GuardEnv::default();
    elim_guards_in_block(stmts, &mut env);
}

fn apply_guard_expr_step(expr: &Expr, env: &mut GuardEnv) {
    match expr {
        Expr::Seq(items) => {
            for item in items {
                apply_guard_expr_step(item, env);
            }
        }
        Expr::Assign(lhs, rhs) => {
            if expr_has_non_panic_call(rhs) || expr_has_non_panic_call(lhs) {
                env.invalidate_members();
            }
            if let Expr::Ident(dst) = lhs.as_ref() {
                let dst = dst.clone();
                let ub = env.expr_upper_bound(rhs);
                let lt_pair = match rhs.as_ref() {
                    Expr::Binary(BinOp::Lt, l, r) => env.resolve_key(l).zip(env.resolve_key(r)),
                    _ => None,
                };
                let alias_key = env.resolve_key(rhs);
                let mut rhs_written = HashSet::new();
                collect_written_idents_expr(rhs, &mut rhs_written);
                for w in rhs_written {
                    env.invalidate(&w);
                }
                env.invalidate(&dst);
                if let Some(hi) = ub {
                    env.upper_bounds.insert(dst.clone(), hi);
                }
                if let Some((a, b)) = lt_pair {
                    if !a.mentions_ident(&dst) && !b.mentions_ident(&dst) {
                        env.bool_lt_def.insert(dst.clone(), (a, b));
                    }
                }
                if let Some(ak) = alias_key {
                    if !ak.mentions_ident(&dst) {
                        env.expr_alias.insert(dst, ak);
                    }
                }
            } else {
                env.invalidate_members();
                let mut written = HashSet::new();
                collect_written_idents_expr(lhs, &mut written);
                collect_written_idents_expr(rhs, &mut written);
                for w in written {
                    env.invalidate(&w);
                }
            }
        }
        other => {
            if expr_has_non_panic_call(other) {
                env.invalidate_members();
            }
            let mut written = HashSet::new();
            collect_written_idents_expr(other, &mut written);
            for w in written {
                env.invalidate(&w);
            }
        }
    }
}

fn apply_guard_decl(name: &str, init: Option<&Expr>, env: &mut GuardEnv) {
    let mut alias_key = None;
    let mut ub = None;
    if let Some(e) = init {
        if expr_has_non_panic_call(e) {
            env.invalidate_members();
        }
        ub = env.expr_upper_bound(e);
        alias_key = env.resolve_key(e);
        let mut written = HashSet::new();
        collect_written_idents_expr(e, &mut written);
        for w in written {
            env.invalidate(&w);
        }
    }
    env.invalidate(name);
    if let Some(hi) = ub {
        env.upper_bounds.insert(name.to_owned(), hi);
    }
    if let Some(ak) = alias_key {
        if !ak.mentions_ident(name) {
            env.expr_alias.insert(name.to_owned(), ak);
        }
    }
}

fn elim_guards_in_block(stmts: &mut Vec<Stmt>, env: &mut GuardEnv) {
    let mut i = 0;
    while i < stmts.len() {
        // Check if `stmts[i]` is a cold diverging guard `if (cond) { panic...; }` whose condition
        // is already proven false.
        let remove = matches!(
            &stmts[i],
            Stmt::If(cond, then_b, None)
                if stmts_definitely_diverge(then_b)
                    && !expr_has_side_effect(cond)
                    && env.guard_is_dead(cond)
        );
        if remove {
            stmts.remove(i);
            continue;
        }

        if let Some(lp) = stmts[i].as_loop_mut() {
            env.invalidate_members();
            let mut written = HashSet::new();
            if let Some(cond) = lp.cond.as_deref() {
                collect_written_idents_expr(cond, &mut written);
            }
            collect_written_idents_stmts(lp.body, &mut written, true);
            for w in written {
                env.invalidate(&w);
            }
            let mut b_env = env.clone();
            if let Some(cond) = lp.cond.as_deref() {
                b_env.record_cond_true(cond);
            }
            elim_guards_in_block(lp.body, &mut b_env);
            i += 1;
            continue;
        }

        match &mut stmts[i] {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                apply_guard_expr_step(e, env);
            }
            Stmt::Let(n, init) => {
                let n = n.clone();
                apply_guard_decl(&n, init.as_ref(), env);
            }
            Stmt::Const(n, e) => {
                let n = n.clone();
                apply_guard_decl(&n, Some(e), env);
            }
            Stmt::Lets(decls) => {
                let mut written = HashSet::new();
                for (n, init) in decls {
                    written.insert(n.clone());
                    if let Some(e) = init {
                        if expr_has_non_panic_call(e) {
                            env.invalidate_members();
                        }
                        collect_written_idents_expr(e, &mut written);
                    }
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::If(cond, then_b, else_b) => {
                if expr_has_non_panic_call(cond) {
                    env.invalidate_members();
                }
                let mut cond_written = HashSet::new();
                collect_written_idents_expr(cond, &mut cond_written);
                for w in cond_written {
                    env.invalidate(&w);
                }
                let then_diverges = stmts_definitely_diverge(then_b);
                let mut then_env = env.clone();
                then_env.record_cond_true(cond);
                elim_guards_in_block(then_b, &mut then_env);

                if let Some(eb) = else_b {
                    let else_diverges = stmts_definitely_diverge(eb);
                    let mut else_env = env.clone();
                    else_env.record_cond_false(cond);
                    elim_guards_in_block(eb, &mut else_env);
                    if else_diverges && !then_diverges {
                        *env = then_env;
                    } else if then_diverges && !else_diverges {
                        *env = else_env;
                    } else {
                        env.invalidate_members();
                        let mut written = HashSet::new();
                        collect_written_idents_stmts(then_b, &mut written, true);
                        collect_written_idents_stmts(eb, &mut written, true);
                        for w in written {
                            env.invalidate(&w);
                        }
                    }
                } else if then_diverges {
                    // Cold diverging guard `if (cond) { panic; }` fell through: `!cond` holds!
                    env.record_cond_false(cond);
                } else {
                    env.invalidate_members();
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(then_b, &mut written, true);
                    for w in written {
                        env.invalidate(&w);
                    }
                }
            }
            Stmt::Switch(disc, cases) => {
                env.invalidate_members();
                let mut written = HashSet::new();
                collect_written_idents_expr(disc, &mut written);
                for w in &written {
                    env.invalidate(w);
                }
                for c in cases.iter_mut() {
                    let mut c_env = env.clone();
                    elim_guards_in_block(&mut c.body, &mut c_env);
                    collect_written_idents_stmts(&c.body, &mut written, true);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::Block(b) => elim_guards_in_block(b, env),
            Stmt::Labeled(_, inner) => match inner.as_mut() {
                Stmt::Block(b) => {
                    env.invalidate_members();
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(b, &mut written, false);
                    let mut b_env = env.clone();
                    elim_guards_in_block(b, &mut b_env);
                    for w in written {
                        env.invalidate(&w);
                    }
                }
                _ => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        &mut written,
                        true,
                    );
                    for w in written {
                        env.invalidate(&w);
                    }
                }
            },
            Stmt::TryCatch { body, handler, .. } => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(body, &mut written, false);
                collect_written_idents_stmts(handler, &mut written, false);
                let mut b_env = env.clone();
                elim_guards_in_block(body, &mut b_env);
                for w in written {
                    env.invalidate(&w);
                }
                let mut h_env = env.clone();
                elim_guards_in_block(handler, &mut h_env);
            }
            _ => {}
        }
        i += 1;
    }
}

// =================================================================================================
// 6. Loop-Versioned BCE (Hot/Cold Pre-Loop Guard)
// =================================================================================================

#[derive(Clone, Default)]
struct VersionEnv {
    /// Variables whose current value is `(... & mask) >>> 0`.
    active_masked: HashMap<String, Expr>,
    /// Variables initialized to exact `u32` constants.
    active_consts: HashMap<String, u32>,
}

impl VersionEnv {
    fn invalidate(&mut self, ident: &str) {
        self.active_masked.remove(ident);
        self.active_masked
            .retain(|_, mask_expr| !expr_reads_ident(mask_expr, ident));
        self.active_consts.remove(ident);
    }

    fn invalidate_all(&mut self, idents: &HashSet<String>) {
        for ident in idents {
            self.invalidate(ident);
        }
    }
}

pub(in super::super) fn version_checked_loops(stmts: &mut [Stmt]) {
    if stmts_have_mutating_nested_fn(stmts) {
        return;
    }
    let mut env = VersionEnv::default();
    version_loops_in_block(stmts, &mut env);
}

fn version_loops_in_block(stmts: &mut [Stmt], env: &mut VersionEnv) {
    // Walk statements in order, maintaining `VersionEnv` so inner loops wrapped in
    // `a: { for (;;) { ... } }` inherit pre-loop facts from their enclosing block, while still
    // versioning inner loops first.
    let no_assigned: HashSet<String> = HashSet::new();
    for i in 0..stmts.len() {
        let pre_loop_env = env.clone();
        if let Some(lp) = stmts[i].as_loop_mut() {
            let mut w = HashSet::new();
            if let Some(c) = lp.cond.as_deref() {
                collect_written_idents_expr(c, &mut w);
            }
            collect_written_idents_stmts(lp.body, &mut w, true);
            env.invalidate_all(&w);
            let mut inner_env = env.clone();
            version_loops_in_block(lp.body, &mut inner_env);
        } else {
            match &mut stmts[i] {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    let mut rhs_w = HashSet::new();
                    collect_written_idents_expr(rhs, &mut rhs_w);
                    env.invalidate_all(&rhs_w);
                    if let Expr::Ident(id) = lhs.as_ref() {
                        let id = id.clone();
                        let mask = extract_bitand_mask(rhs, &no_assigned)
                            .filter(|m| !expr_reads_ident(m, &id));
                        let c_val = exact_u32(rhs);
                        env.invalidate(&id);
                        if let Some(m) = mask {
                            env.active_masked.insert(id.clone(), m);
                        }
                        if let Some(c) = c_val {
                            env.active_consts.insert(id, c);
                        }
                    } else {
                        let mut lhs_w = HashSet::new();
                        collect_written_idents_expr(lhs, &mut lhs_w);
                        env.invalidate_all(&lhs_w);
                    }
                }
                Stmt::If(cond, a, b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_expr(cond, &mut w);
                    env.invalidate_all(&w);
                    let mut a_env = env.clone();
                    version_loops_in_block(a, &mut a_env);
                    collect_written_idents_stmts(a, &mut w, true);
                    if let Some(b) = b {
                        let mut b_env = env.clone();
                        version_loops_in_block(b, &mut b_env);
                        collect_written_idents_stmts(b, &mut w, true);
                    }
                    env.invalidate_all(&w);
                }
                Stmt::Switch(disc, cases) => {
                    let mut w = HashSet::new();
                    collect_written_idents_expr(disc, &mut w);
                    env.invalidate_all(&w);
                    for c in cases {
                        let mut c_env = env.clone();
                        version_loops_in_block(&mut c.body, &mut c_env);
                        collect_written_idents_stmts(&c.body, &mut w, true);
                    }
                    env.invalidate_all(&w);
                }
                Stmt::Block(b) => version_loops_in_block(b, env),
                Stmt::Labeled(_, inner) => match inner.as_mut() {
                    Stmt::Block(b) => {
                        let mut w = HashSet::new();
                        collect_written_idents_stmts(b, &mut w, false);
                        let mut b_env = env.clone();
                        version_loops_in_block(b, &mut b_env);
                        env.invalidate_all(&w);
                    }
                    other => {
                        let mut inner_env = env.clone();
                        version_loops_in_block(std::slice::from_mut(other), &mut inner_env);
                        let mut w = HashSet::new();
                        collect_written_idents_stmts(std::slice::from_ref(other), &mut w, true);
                        env.invalidate_all(&w);
                    }
                },
                Stmt::TryCatch { body, handler, .. } => {
                    let mut b_env = env.clone();
                    version_loops_in_block(body, &mut b_env);
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(body, &mut w, false);
                    env.invalidate_all(&w);
                    let mut h_env = env.clone();
                    version_loops_in_block(handler, &mut h_env);
                    collect_written_idents_stmts(handler, &mut w, false);
                    env.invalidate_all(&w);
                }
                other => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(std::slice::from_ref(other), &mut w, true);
                    env.invalidate_all(&w);
                }
            }
        }

        if let Some(versioned) = try_version_loop_at(stmts, i, &pre_loop_env) {
            stmts[i] = versioned;
        }
    }
}

/// Extracts `mask` from `(x & mask)` or `(x & mask) >>> 0` where `mask` is either a constant
/// `0..=0x7fffffff` or a loop-invariant identifier.
fn extract_bitand_mask(expr: &Expr, assigned_in_loop: &HashSet<String>) -> Option<Expr> {
    let inner = match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => l.as_ref(),
        other => other,
    };
    let Expr::Binary(BinOp::BitAnd, l, r) = inner else {
        return None;
    };
    let valid_mask = |e: &Expr| -> bool {
        match e {
            Expr::Ident(id) => !assigned_in_loop.contains(id),
            other => exact_u32(other).is_some_and(|m| m <= i32::MAX as u32),
        }
    };
    if valid_mask(r) {
        Some(r.as_ref().clone())
    } else if valid_mask(l) {
        Some(l.as_ref().clone())
    } else {
        None
    }
}

/// Detects loop-carried variables like `q` in `earcut/refine` where the pre-loop definition AND
/// every in-loop write are `(... & mask) >>> 0` with the same loop-invariant `mask`.
fn detect_loop_masked_vars(
    enclosing: &[Stmt],
    loop_idx: usize,
    loop_body: &[Stmt],
    assigned_in_loop: &HashSet<String>,
    pre_loop_masked: &HashMap<String, Expr>,
) -> HashMap<String, Expr> {
    struct MaskScan<'a> {
        assigned_in_loop: &'a HashSet<String>,
        masks: HashMap<String, Option<Expr>>,
    }
    impl Visit for MaskScan<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if let Expr::Ident(id) = lhs.as_ref() {
                        let m = extract_bitand_mask(rhs, self.assigned_in_loop);
                        self.masks
                            .entry(id.clone())
                            .and_modify(|cur| {
                                if *cur != m {
                                    *cur = None;
                                }
                            })
                            .or_insert(m);
                    }
                }
                Stmt::If(_, a, b) => {
                    if !stmts_definitely_diverge(a) {
                        self.visit_stmts(a);
                    }
                    if let Some(b) = b {
                        if !stmts_definitely_diverge(b) {
                            self.visit_stmts(b);
                        }
                    }
                }
                Stmt::TryCatch { .. } | Stmt::FunctionDecl { .. } => {}
                _ => jsast::walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &Expr) {}
    }

    let mut scan = MaskScan {
        assigned_in_loop,
        masks: HashMap::new(),
    };
    scan.visit_stmts(loop_body);

    let mut confirmed = HashMap::new();
    for (var, maybe_mask) in scan.masks {
        let Some(mask_expr) = maybe_mask else {
            continue;
        };
        if pre_loop_masked.get(&var) == Some(&mask_expr) {
            confirmed.insert(var.clone(), mask_expr);
            continue;
        }
        // Fallback: scan up to 6 preceding statements in the same block.
        let mut k = loop_idx;
        let mut steps = 0;
        while k > 0 && steps < 6 {
            k -= 1;
            steps += 1;
            if let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = &enclosing[k] {
                if matches!(lhs.as_ref(), Expr::Ident(id) if id == &var) {
                    if extract_bitand_mask(rhs, assigned_in_loop).as_ref() == Some(&mask_expr) {
                        confirmed.insert(var.clone(), mask_expr);
                    }
                    break;
                }
            }
        }
    }
    confirmed
}

fn is_panic_bounds_check_if(
    stmt: &Stmt,
    local_defs: &HashMap<String, Expr>,
) -> Option<(Expr, Expr)> {
    let Stmt::If(cond, then_b, None) = stmt else {
        return None;
    };
    let is_bounds_panic = then_b.iter().any(|s| match s {
        Stmt::ExprStmt(Expr::Call(callee, _)) | Stmt::Throw(Expr::Call(callee, _)) => matches!(
            callee.as_ref(),
            Expr::Ident(n) if n.starts_with("core$panicking$panic_bounds_check$")
        ),
        _ => false,
    });
    if !is_bounds_panic || !stmts_definitely_diverge(then_b) {
        return None;
    }
    let Expr::Unary(UnOp::Not, inner) = cond else {
        return None;
    };
    match inner.as_ref() {
        Expr::Binary(BinOp::Lt, l, r) => Some((l.as_ref().clone(), r.as_ref().clone())),
        Expr::Ident(flag) => {
            if let Some(Expr::Binary(BinOp::Lt, l, r)) = local_defs.get(flag) {
                Some((l.as_ref().clone(), r.as_ref().clone()))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Resolves an expression into an affine form `scale * iv + offset` (exact over u32 without wrap
/// when guards hold), walking through `local_defs` up to depth 10.
fn resolve_affine_iv(
    expr: &Expr,
    iv: &str,
    local_defs: &HashMap<String, Expr>,
    depth: u8,
) -> Option<(i64, i64)> {
    if depth >= 10 {
        return None;
    }
    let inner = match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => l.as_ref(),
        Expr::Binary(BinOp::BitOr, l, r) if exact_i32(r) == Some(0) => l.as_ref(),
        other => other,
    };
    if let Expr::Ident(id) = inner {
        if id == iv {
            return Some((1, 0));
        }
        if let Some(def) = local_defs.get(id) {
            return resolve_affine_iv(def, iv, local_defs, depth + 1);
        }
        return None;
    }
    match inner {
        Expr::Binary(BinOp::Add, l, r) => {
            if let Some(c) = exact_u32(r) {
                let (s, o) = resolve_affine_iv(l, iv, local_defs, depth + 1)?;
                return Some((s, o + i64::from(c)));
            }
            if let Some(c) = exact_u32(l) {
                let (s, o) = resolve_affine_iv(r, iv, local_defs, depth + 1)?;
                return Some((s, o + i64::from(c)));
            }
            None
        }
        Expr::Binary(BinOp::Sub, l, r) => {
            let c = exact_u32(r)?;
            let (s, o) = resolve_affine_iv(l, iv, local_defs, depth + 1)?;
            Some((s, o - i64::from(c)))
        }
        Expr::Call(callee, args) if args.len() == 2 => {
            let is_imul = matches!(
                callee.as_ref(),
                Expr::Member(obj, prop) if prop == "imul" && matches!(obj.as_ref(), Expr::Ident(m) if m == "Math")
            );
            if !is_imul {
                return None;
            }
            if let Some(k) = exact_u32(&args[1]) {
                if (1..=64).contains(&k) {
                    let (s, o) = resolve_affine_iv(&args[0], iv, local_defs, depth + 1)?;
                    return Some((s * i64::from(k), o * i64::from(k)));
                }
            }
            if let Some(k) = exact_u32(&args[0]) {
                if (1..=64).contains(&k) {
                    let (s, o) = resolve_affine_iv(&args[1], iv, local_defs, depth + 1)?;
                    return Some((s * i64::from(k), o * i64::from(k)));
                }
            }
            None
        }
        _ => None,
    }
}

/// Resolves `base + iv >>> 0`, `(base + iv >>> 0) + C >>> 0`, or `(base + iv | 0) - sub >>> 0`.
enum BaseIvShape {
    /// `base + iv >>> 0`
    Add { base: String },
    /// `(base + iv >>> 0) + add >>> 0` with `1 <= add <= 64`
    AddConst { base: String, add: u32 },
    /// `(base + iv | 0) - sub >>> 0`
    AddSub { base: String, sub: String },
}

fn resolve_base_iv_shape(
    expr: &Expr,
    iv: &str,
    local_defs: &HashMap<String, Expr>,
    assigned_in_loop: &HashSet<String>,
) -> Option<BaseIvShape> {
    let e = match expr {
        Expr::Ident(id) => local_defs.get(id)?,
        other => other,
    };
    let inner = match e {
        Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => l.as_ref(),
        other => other,
    };
    let match_base_plus_iv = |node: &Expr| -> Option<String> {
        let resolved = match node {
            Expr::Ident(id) => local_defs.get(id).unwrap_or(node),
            other => other,
        };
        let node = match resolved {
            Expr::Binary(BinOp::BitOr, l, r) if exact_i32(r) == Some(0) => l.as_ref(),
            Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => l.as_ref(),
            other => other,
        };
        let Expr::Binary(BinOp::Add, l, r) = node else {
            return None;
        };
        match (l.as_ref(), r.as_ref()) {
            (Expr::Ident(b), Expr::Ident(v))
                if v == iv && b != iv && !assigned_in_loop.contains(b) =>
            {
                Some(b.clone())
            }
            (Expr::Ident(v), Expr::Ident(b))
                if v == iv && b != iv && !assigned_in_loop.contains(b) =>
            {
                Some(b.clone())
            }
            _ => None,
        }
    };
    if let Some(base) = match_base_plus_iv(inner) {
        return Some(BaseIvShape::Add { base });
    }
    if let Expr::Binary(BinOp::Add, l, r) = inner {
        if let (Some(base), Some(add)) = (match_base_plus_iv(l), exact_u32(r)) {
            if (1..=64).contains(&add) {
                return Some(BaseIvShape::AddConst { base, add });
            }
        }
        if let (Some(add), Some(base)) = (exact_u32(l), match_base_plus_iv(r)) {
            if (1..=64).contains(&add) {
                return Some(BaseIvShape::AddConst { base, add });
            }
        }
    }
    if let Expr::Binary(BinOp::Sub, l, r) = inner {
        if let (Some(base), Expr::Ident(sub)) = (match_base_plus_iv(l), r.as_ref()) {
            if sub != iv && !assigned_in_loop.contains(sub) {
                return Some(BaseIvShape::AddSub {
                    base,
                    sub: sub.clone(),
                });
            }
        }
    }
    None
}

fn derive_affine_guards(scale: i64, offset: i64, c: &CounterInfo, len: &Expr) -> Option<Vec<Expr>> {
    if scale < 1 {
        return None;
    }
    let mut guards = Vec::new();
    // Lower-bound guard (no underflow on `iv + offset`).
    let need_min: i64 = if offset < 0 {
        // `scale * iv + offset >= 0` -> `iv >= ceil(-offset / scale)`
        (-offset + scale - 1) / scale
    } else {
        0
    };
    if c.iv_min.is_none_or(|m| i64::from(m) < need_min) {
        guards.push(Expr::Binary(
            BinOp::Le,
            Box::new(Expr::Num(need_min as f64)),
            Box::new(Expr::Ident(c.iv.clone())),
        ));
    }
    if let Some(b_const) = exact_u32(&c.bound) {
        if b_const == 0 {
            return Some(Vec::new());
        }
        let max_val = scale
            .checked_mul(i64::from(b_const) - 1)?
            .checked_add(offset)?;
        if max_val < 0 || max_val > i64::from(u32::MAX) {
            return None;
        }
        if let Some(lc) = exact_u32(len) {
            return (max_val < i64::from(lc)).then_some(guards);
        }
        guards.push(Expr::Binary(
            BinOp::Lt,
            Box::new(Expr::Num(max_val as f64)),
            Box::new(len.clone()),
        ));
        return Some(guards);
    }
    if scale == 1 && offset <= 0 {
        // `iv + offset < bound <= len`
        guards.push(Expr::Binary(
            BinOp::Le,
            Box::new(c.bound.clone()),
            Box::new(len.clone()),
        ));
        return Some(guards);
    }
    None
}

fn derive_base_iv_guards(shape: BaseIvShape, c: &CounterInfo, len: &Expr) -> Vec<Expr> {
    let mut guards = Vec::new();
    if c.iv_min.is_none() {
        guards.push(Expr::Binary(
            BinOp::Le,
            Box::new(Expr::Num(0.0)),
            Box::new(Expr::Ident(c.iv.clone())),
        ));
    }
    match shape {
        BaseIvShape::Add { base } => {
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(Expr::Num(0.0)),
                Box::new(Expr::Ident(base.clone())),
            ));
            let sum = Expr::Binary(
                BinOp::Add,
                Box::new(Expr::Ident(base)),
                Box::new(c.bound.clone()),
            );
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(sum.clone()),
                Box::new(len.clone()),
            ));
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(sum),
                Box::new(Expr::Num(4294967296.0)),
            ));
        }
        BaseIvShape::AddConst { base, add } => {
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(Expr::Num(0.0)),
                Box::new(Expr::Ident(base.clone())),
            ));
            let base_plus_bound = Expr::Binary(
                BinOp::Add,
                Box::new(Expr::Ident(base)),
                Box::new(c.bound.clone()),
            );
            let max_idx = if add == 1 {
                base_plus_bound
            } else {
                Expr::Binary(
                    BinOp::Add,
                    Box::new(base_plus_bound),
                    Box::new(Expr::Num(f64::from(add - 1))),
                )
            };
            guards.push(Expr::Binary(
                BinOp::Lt,
                Box::new(max_idx.clone()),
                Box::new(len.clone()),
            ));
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(max_idx),
                Box::new(Expr::Num(4294967295.0)),
            ));
        }
        BaseIvShape::AddSub { base, sub } => {
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(Expr::Num(0.0)),
                Box::new(Expr::Ident(base.clone())),
            ));
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(Expr::Num(0.0)),
                Box::new(Expr::Ident(sub.clone())),
            ));
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(Expr::Ident(sub.clone())),
                Box::new(Expr::Ident(base.clone())),
            ));
            let sum = Expr::Binary(
                BinOp::Add,
                Box::new(Expr::Ident(base)),
                Box::new(c.bound.clone()),
            );
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(sum.clone()),
                Box::new(Expr::Num(2147483648.0)),
            ));
            let diff = Expr::Binary(BinOp::Sub, Box::new(sum), Box::new(Expr::Ident(sub)));
            guards.push(Expr::Binary(
                BinOp::Le,
                Box::new(diff),
                Box::new(len.clone()),
            ));
        }
    }
    guards
}

fn simplify_guards(all_guards: Vec<Expr>) -> Vec<Expr> {
    // First pass: record the strictest constant LHS for `K < rhs` (keep max K) and `C <= rhs`
    // (keep max C), and strictest constant RHS for `lhs <= C` (keep min C).
    let mut max_lt: Vec<(Expr, u32)> = Vec::new();
    let mut max_le: Vec<(Expr, u32)> = Vec::new();
    let mut min_le_rhs: Vec<(Expr, u64)> = Vec::new();
    for g in &all_guards {
        match g {
            Expr::Binary(BinOp::Lt, l, r) => {
                if let Some(k) = exact_u32(l) {
                    if let Some((_, cur)) = max_lt.iter_mut().find(|(e, _)| e == r.as_ref()) {
                        *cur = (*cur).max(k);
                    } else {
                        max_lt.push((r.as_ref().clone(), k));
                    }
                }
            }
            Expr::Binary(BinOp::Le, l, r) => {
                if let Some(c) = exact_u32(l) {
                    if let Some((_, cur)) = max_le.iter_mut().find(|(e, _)| e == r.as_ref()) {
                        *cur = (*cur).max(c);
                    } else {
                        max_le.push((r.as_ref().clone(), c));
                    }
                }
                if let Some(c) = exact_u64(r) {
                    if let Some((_, cur)) = min_le_rhs.iter_mut().find(|(e, _)| e == l.as_ref()) {
                        *cur = (*cur).min(c);
                    } else {
                        min_le_rhs.push((l.as_ref().clone(), c));
                    }
                }
            }
            _ => {}
        }
    }

    let mut deduped: Vec<Expr> = Vec::new();
    for g in &all_guards {
        if let Expr::Binary(BinOp::Le, l, r) = g {
            // If `all_guards` already contains strict `l < r`, drop weaker `l <= r`.
            let has_strict = all_guards
                .iter()
                .any(|o| matches!(o, Expr::Binary(BinOp::Lt, ol, or) if ol == l && or == r));
            if has_strict {
                continue;
            }
        }
        if let Expr::Binary(BinOp::Lt, l, r) = g {
            if let Some(k) = exact_u32(l) {
                if let Some((_, max_k)) = max_lt.iter().find(|(e, _)| e == r.as_ref()) {
                    if k < *max_k {
                        continue;
                    }
                }
            }
        } else if let Expr::Binary(BinOp::Le, l, r) = g {
            if let Some(c) = exact_u32(l) {
                if let Some((_, max_c)) = max_le.iter().find(|(e, _)| e == r.as_ref()) {
                    if c < *max_c {
                        continue;
                    }
                }
            }
            if let Some(c) = exact_u64(r) {
                if let Some((_, min_c)) = min_le_rhs.iter().find(|(e, _)| e == l.as_ref()) {
                    if c > *min_c {
                        continue;
                    }
                }
            }
        }
        if !deduped.contains(g) {
            deduped.push(g.clone());
        }
    }
    deduped
}

fn try_version_loop_at(
    enclosing: &[Stmt],
    loop_idx: usize,
    pre_loop_env: &VersionEnv,
) -> Option<Stmt> {
    let body = enclosing[loop_idx].as_loop()?.body;
    if count_stmts(body) > 180 {
        return None;
    }

    let mut assigned_in_loop = HashSet::new();
    collect_written_idents_stmts(body, &mut assigned_in_loop, true);
    let counter = detect_loop_counter(
        enclosing,
        loop_idx,
        &assigned_in_loop,
        Some(&pre_loop_env.active_consts),
    );
    let loop_masked = detect_loop_masked_vars(
        enclosing,
        loop_idx,
        body,
        &assigned_in_loop,
        &pre_loop_env.active_masked,
    );

    // Clone the loop and strip all bounds checks we can prove with invariant pre-loop guards.
    let mut fast_loop = enclosing[loop_idx].clone();
    let mut stripper = CheckStripper {
        counter: counter.as_ref(),
        loop_masked: &loop_masked,
        assigned_in_loop: &assigned_in_loop,
        all_guards: Vec::new(),
        removed: 0,
    };

    let fast_body = fast_loop.as_loop_mut()?.body;
    let mut local_defs: HashMap<String, Expr> = HashMap::new();
    stripper.strip_in_block(fast_body, &mut local_defs);

    if stripper.removed == 0 {
        return None;
    }

    let mut deduped = simplify_guards(stripper.all_guards);
    if deduped.is_empty() {
        // Every check proved unconditionally! No versioning needed.
        return Some(fast_loop);
    }

    let mut guard_expr = deduped.remove(0);
    for g in deduped {
        guard_expr = Expr::Binary(BinOp::And, Box::new(guard_expr), Box::new(g));
    }
    Some(Stmt::If(
        guard_expr,
        vec![fast_loop],
        Some(vec![enclosing[loop_idx].clone()]),
    ))
}

fn record_local_assign_expr(expr: &Expr, local_defs: &mut HashMap<String, Expr>) {
    match expr {
        Expr::Assign(lhs, rhs) => {
            if let Expr::Ident(id) = lhs.as_ref() {
                let id = id.clone();
                local_defs.retain(|k, v| k != &id && !expr_reads_ident(v, &id));
                if !expr_has_side_effect(rhs) && !expr_reads_ident(rhs, &id) {
                    local_defs.insert(id, rhs.as_ref().clone());
                }
            } else {
                let mut w = HashSet::new();
                collect_written_idents_expr(expr, &mut w);
                local_defs
                    .retain(|k, v| !w.contains(k) && !w.iter().any(|x| expr_reads_ident(v, x)));
            }
        }
        Expr::Seq(items) => {
            for item in items {
                record_local_assign_expr(item, local_defs);
            }
        }
        other => {
            let mut w = HashSet::new();
            collect_written_idents_expr(other, &mut w);
            local_defs.retain(|k, v| !w.contains(k) && !w.iter().any(|x| expr_reads_ident(v, x)));
        }
    }
}

struct CheckStripper<'a> {
    counter: Option<&'a CounterInfo>,
    loop_masked: &'a HashMap<String, Expr>,
    assigned_in_loop: &'a HashSet<String>,
    all_guards: Vec<Expr>,
    removed: usize,
}

impl CheckStripper<'_> {
    fn derive_check_guards(
        &self,
        idx: &Expr,
        len: &Expr,
        local_defs: &HashMap<String, Expr>,
    ) -> Option<Vec<Expr>> {
        // Resolve `len` through up to 4 hops of `local_defs` (e.g. `ao = $hl_0; N = ao;`), then
        // verify `len` is a constant u32 or a loop-invariant identifier.
        let mut resolved_len = len.clone();
        for _ in 0..4 {
            let Expr::Ident(id) = &resolved_len else {
                break;
            };
            if !self.assigned_in_loop.contains(id) {
                break;
            }
            resolved_len = local_defs.get(id)?.clone();
        }
        let len = &resolved_len;
        let len_inv = match len {
            Expr::Ident(id) => !self.assigned_in_loop.contains(id),
            other => exact_u32(other).is_some(),
        };
        if !len_inv {
            return None;
        }

        // 1. Constant index `K < len`.
        if let Some(k) = exact_u32(idx) {
            if let Some(lc) = exact_u32(len) {
                return (k < lc).then(Vec::new);
            }
            return Some(vec![Expr::Binary(
                BinOp::Lt,
                Box::new(Expr::Num(f64::from(k))),
                Box::new(len.clone()),
            )]);
        }

        // 2. Bitmask index (`... & mask` directly, via local def, or loop-carried masked var).
        let mask_opt = extract_bitand_mask(idx, self.assigned_in_loop).or_else(|| match idx {
            Expr::Ident(id) => self.loop_masked.get(id).cloned().or_else(|| {
                local_defs
                    .get(id)
                    .and_then(|d| extract_bitand_mask(d, self.assigned_in_loop))
            }),
            _ => None,
        });
        if let Some(mask_expr) = mask_opt {
            if let (Some(mc), Some(lc)) = (exact_u32(&mask_expr), exact_u32(len)) {
                return (mc < lc).then(Vec::new);
            }
            let lhs = if exact_u32(&mask_expr).is_some() {
                mask_expr
            } else {
                Expr::Binary(BinOp::UShr, Box::new(mask_expr), Box::new(Expr::Num(0.0)))
            };
            return Some(vec![Expr::Binary(
                BinOp::Lt,
                Box::new(lhs),
                Box::new(len.clone()),
            )]);
        }

        let c = self.counter?;

        // 3. Affine in `iv`: `scale * iv + offset`.
        if let Some((scale, offset)) = resolve_affine_iv(idx, &c.iv, local_defs, 0) {
            if let Some(guards) = derive_affine_guards(scale, offset, c, len) {
                return Some(guards);
            }
        }

        // 4. `base + iv >>> 0`, `(base + iv >>> 0) + C >>> 0`, or `(base + iv | 0) - sub >>> 0`.
        if let Some(shape) = resolve_base_iv_shape(idx, &c.iv, local_defs, self.assigned_in_loop) {
            return Some(derive_base_iv_guards(shape, c, len));
        }

        None
    }

    fn strip_in_block(&mut self, stmts: &mut Vec<Stmt>, local_defs: &mut HashMap<String, Expr>) {
        let mut i = 0;
        while i < stmts.len() {
            if let Some((idx, len)) = is_panic_bounds_check_if(&stmts[i], local_defs) {
                if let Some(guards) = self.derive_check_guards(&idx, &len, local_defs) {
                    self.all_guards.extend(guards);
                    stmts.remove(i);
                    self.removed += 1;
                    continue;
                }
            }
            match &mut stmts[i] {
                Stmt::ExprStmt(e) => record_local_assign_expr(e, local_defs),
                Stmt::If(_, a, b) => {
                    if !stmts_definitely_diverge(a) {
                        let mut a_defs = local_defs.clone();
                        self.strip_in_block(a, &mut a_defs);
                    }
                    if let Some(b) = b {
                        if !stmts_definitely_diverge(b) {
                            let mut b_defs = local_defs.clone();
                            self.strip_in_block(b, &mut b_defs);
                        }
                    }
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(a, &mut written, true);
                    if let Some(b) = b {
                        collect_written_idents_stmts(b, &mut written, true);
                    }
                    local_defs.retain(|k, v| {
                        !written.contains(k) && !written.iter().any(|w| expr_reads_ident(v, w))
                    });
                }
                Stmt::Block(b) => self.strip_in_block(b, local_defs),
                Stmt::Labeled(_, inner) => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        &mut written,
                        false,
                    );
                    if let Stmt::Block(b) = inner.as_mut() {
                        let mut b_defs = local_defs.clone();
                        self.strip_in_block(b, &mut b_defs);
                    }
                    local_defs.retain(|k, v| {
                        !written.contains(k) && !written.iter().any(|w| expr_reads_ident(v, w))
                    });
                }
                Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(b, &mut written, true);
                    local_defs.retain(|k, v| {
                        !written.contains(k) && !written.iter().any(|w| expr_reads_ident(v, w))
                    });
                }
                _ => {}
            }
            i += 1;
        }
    }
}
