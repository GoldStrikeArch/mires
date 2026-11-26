//! Flow-sensitive numeric/integer domain tracking (`NumEnv`) and unsigned range check folding
//! (`(x >= A && x <= B)` -> `(x - A >>> 0) <= (B - A)`).

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp, Visit};

use super::super::ast_util::{
    collect_written_idents_expr, collect_written_idents_stmt, collect_written_idents_stmts,
    exact_i32, exact_u32, stmts_definitely_exit,
};
use super::fold::{exact_f64_lit, is_pos_zero, is_pure_index_expr};
use super::tally_ident_reads;

// =============================================================================================
// Flow-sensitive numeric/integer domain tracking (shared by range folding & ASCII LUTs)
// =============================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NumKind {
    /// Exact integer in `[0, 0x7fff_ffff]` (never `NaN`, fractional, negative, or `>= 2^31`).
    NonNegI31,
    /// Exact integer in `[0, 0xffff_ffff]` (`Uint32`, never `NaN` or fractional).
    U32,
    /// Exact integer in `[-2^31, 2^31 - 1]` (`Int32`, never `NaN` or fractional).
    I32,
    /// Value in `{0..=0x7fff_ffff} ∪ {NaN}` (e.g. `s.charCodeAt(idx)` without locally proven bounds).
    NonNegI31OrNan,
    /// Guaranteed JS `number` (may be float / `NaN` / `Infinity`, NEVER string/object/bool).
    Numeric,
}

impl NumKind {
    fn join(self, other: Self) -> Self {
        if self == other {
            return self;
        }
        match (self, other) {
            (Self::NonNegI31, Self::U32) | (Self::U32, Self::NonNegI31) => Self::U32,
            (Self::NonNegI31, Self::I32) | (Self::I32, Self::NonNegI31) => Self::I32,
            (Self::NonNegI31, Self::NonNegI31OrNan) | (Self::NonNegI31OrNan, Self::NonNegI31) => {
                Self::NonNegI31OrNan
            }
            _ => Self::Numeric,
        }
    }

    fn is_exact_int32_or_u32(self) -> bool {
        matches!(self, Self::NonNegI31 | Self::U32 | Self::I32)
    }

    fn is_nonneg_int(self) -> bool {
        matches!(self, Self::NonNegI31 | Self::U32)
    }
}

/// Collects simple `v = rhs` assignments inside a loop body while disqualifying variables with
/// compound assignments, local redeclarations, or writes inside nested closures/functions.
#[derive(Default)]
struct LoopWriteScan {
    in_closure: bool,
    rhs_map: HashMap<String, Vec<Expr>>,
    disq: HashSet<String>,
}

impl Visit for LoopWriteScan {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, _) | Stmt::Const(n, _) => {
                self.disq.insert(n.clone());
                jsast::walk_stmt(self, stmt);
            }
            Stmt::Lets(decls) => {
                for (n, _) in decls {
                    self.disq.insert(n.clone());
                }
                jsast::walk_stmt(self, stmt);
            }
            Stmt::TryCatch { binding, .. } => {
                self.disq.insert(binding.clone());
                jsast::walk_stmt(self, stmt);
            }
            Stmt::FunctionDecl { name, body, .. } => {
                self.disq.insert(name.clone());
                let prev = self.in_closure;
                self.in_closure = true;
                self.visit_stmts(body);
                self.in_closure = prev;
            }
            _ => jsast::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(id) = lhs.as_ref() {
                    if self.in_closure {
                        self.disq.insert(id.clone());
                    } else {
                        self.rhs_map
                            .entry(id.clone())
                            .or_default()
                            .push((**rhs).clone());
                    }
                } else {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Ident(id) = lhs.as_ref() {
                    self.disq.insert(id.clone());
                } else {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            Expr::Arrow(_, _) => {
                let prev = self.in_closure;
                self.in_closure = true;
                jsast::walk_expr(self, expr);
                self.in_closure = prev;
            }
            _ => jsast::walk_expr(self, expr),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct NumEnv {
    vars: HashMap<String, NumKind>,
    /// `len_var -> str_var` when `len_var = str_var.length`.
    str_lens: HashMap<String, String>,
    /// Pairs `(idx_var, str_var)` proven `0 <= idx_var < str_var.length`.
    in_bounds: HashSet<(String, String)>,
    /// String vars proven `s.length > 0`.
    nonempty_strs: HashSet<String>,
}

impl NumEnv {
    pub(super) fn invalidate(&mut self, var: &str) {
        self.vars.remove(var);
        self.str_lens.remove(var);
        self.str_lens.retain(|_, s| s != var);
        self.in_bounds.retain(|(i, s)| i != var && s != var);
        self.nonempty_strs.remove(var);
    }

    pub(super) fn invalidate_written_expr(&mut self, expr: &Expr) {
        let mut written = HashSet::new();
        collect_written_idents_expr(expr, &mut written);
        for w in written {
            self.invalidate(&w);
        }
    }

    pub(super) fn invalidate_written_stmts(&mut self, stmts: &[Stmt]) {
        let mut written = HashSet::new();
        collect_written_idents_stmts(stmts, &mut written);
        for w in written {
            self.invalidate(&w);
        }
    }

    fn is_nonneg_int(&self, var: &str) -> bool {
        self.vars.get(var).is_some_and(|k| k.is_nonneg_int())
    }

    fn is_nonneg_i31_expr(&self, expr: &Expr) -> bool {
        self.classify_expr(expr) == Some(NumKind::NonNegI31)
    }

    fn resolve_str_len(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Member(obj, prop) if prop == "length" => match obj.as_ref() {
                Expr::Ident(s) => Some(s.clone()),
                _ => None,
            },
            Expr::Ident(d) => self.str_lens.get(d).cloned(),
            _ => None,
        }
    }

    fn is_char_code_at_in_bounds(&self, recv: &Expr, idx: &Expr) -> bool {
        match recv {
            Expr::Str(lit) => {
                exact_u32(idx).is_some_and(|k| (k as usize) < lit.encode_utf16().count())
            }
            Expr::Ident(s) => {
                if is_pos_zero(idx) && self.nonempty_strs.contains(s) {
                    return true;
                }
                if let Expr::Ident(iv) = idx {
                    if self.in_bounds.contains(&(iv.clone(), s.clone())) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub(super) fn classify_expr(&self, expr: &Expr) -> Option<NumKind> {
        match expr {
            Expr::Ident(id) => self.vars.get(id).copied(),
            Expr::Num(n) => {
                if n.is_finite() && n.fract() == 0.0 {
                    if *n >= 0.0 && *n <= 2147483647.0 {
                        Some(NumKind::NonNegI31)
                    } else if *n >= 0.0 && *n <= 4294967295.0 {
                        Some(NumKind::U32)
                    } else if *n >= -2147483648.0 && *n <= 2147483647.0 {
                        Some(NumKind::I32)
                    } else {
                        Some(NumKind::Numeric)
                    }
                } else {
                    Some(NumKind::Numeric)
                }
            }
            Expr::RawNum(s) => {
                if let Ok(v) = s.parse::<i64>() {
                    if (0..=0x7fff_ffff).contains(&v) {
                        Some(NumKind::NonNegI31)
                    } else if (0..=0xffff_ffff).contains(&v) {
                        Some(NumKind::U32)
                    } else if (-0x8000_0000..=0x7fff_ffff).contains(&v) {
                        Some(NumKind::I32)
                    } else {
                        Some(NumKind::Numeric)
                    }
                } else {
                    Some(NumKind::Numeric)
                }
            }
            Expr::Unary(UnOp::BitNot, _) => Some(NumKind::I32),
            Expr::Unary(UnOp::Neg, _) => {
                if let Some(v) = exact_i32(expr) {
                    Some(if v >= 0 {
                        NumKind::NonNegI31
                    } else {
                        NumKind::I32
                    })
                } else {
                    Some(NumKind::Numeric)
                }
            }
            Expr::Unary(UnOp::Pos, _) => Some(NumKind::Numeric),
            Expr::Binary(BinOp::UShr, _, r) => {
                if exact_u32(r).is_some_and(|k| (k & 31) >= 1) {
                    Some(NumKind::NonNegI31)
                } else {
                    Some(NumKind::U32)
                }
            }
            Expr::Binary(BinOp::BitAnd, l, r) => {
                if self.is_nonneg_i31_expr(l) || self.is_nonneg_i31_expr(r) {
                    Some(NumKind::NonNegI31)
                } else {
                    Some(NumKind::I32)
                }
            }
            Expr::Binary(BinOp::BitOr | BinOp::BitXor, l, r) => {
                if self.is_nonneg_i31_expr(l) && self.is_nonneg_i31_expr(r) {
                    Some(NumKind::NonNegI31)
                } else {
                    Some(NumKind::I32)
                }
            }
            Expr::Binary(BinOp::Shl | BinOp::Shr, _, _) => Some(NumKind::I32),
            Expr::Binary(BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, _, _) => {
                Some(NumKind::Numeric)
            }
            Expr::Binary(BinOp::Add, l, r) => {
                if self.classify_expr(l).is_some() && self.classify_expr(r).is_some() {
                    Some(NumKind::Numeric)
                } else {
                    None
                }
            }
            Expr::Call(callee, args) => match callee.as_ref() {
                Expr::Member(obj, prop) => {
                    if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") {
                        if prop == "clz32" && args.len() == 1 {
                            return Some(NumKind::NonNegI31);
                        }
                        if prop == "imul" && args.len() == 2 {
                            return Some(NumKind::I32);
                        }
                    }
                    if prop == "charCodeAt"
                        && args.len() == 1
                        && matches!(obj.as_ref(), Expr::Ident(_) | Expr::Str(_))
                    {
                        return Some(if self.is_char_code_at_in_bounds(obj, &args[0]) {
                            NumKind::NonNegI31
                        } else {
                            NumKind::NonNegI31OrNan
                        });
                    }
                    None
                }
                _ => None,
            },
            Expr::Cond(_, t, e) => match (self.classify_expr(t), self.classify_expr(e)) {
                (Some(a), Some(b)) => Some(a.join(b)),
                _ => None,
            },
            _ => None,
        }
    }

    pub(super) fn update_assign(&mut self, dst: &str, rhs: &Expr) {
        let kind = self.classify_expr(rhs);
        let str_len_src = match rhs {
            Expr::Member(obj, prop) if prop == "length" => match obj.as_ref() {
                Expr::Ident(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        };
        self.invalidate(dst);
        if let Some(k) = kind {
            self.vars.insert(dst.to_owned(), k);
        }
        if let Some(s) = str_len_src {
            if s != dst {
                self.str_lens.insert(dst.to_owned(), s);
            }
        }
    }

    pub(super) fn apply_cond_facts(&mut self, cond: &Expr, truth: bool) {
        match (cond, truth) {
            (Expr::Unary(UnOp::Not, inner), t) => self.apply_cond_facts(inner, !t),
            (Expr::Binary(BinOp::And, l, r), true) => {
                self.apply_cond_facts(l, true);
                self.apply_cond_facts(r, true);
            }
            (Expr::Binary(BinOp::Or, l, r), false) => {
                self.apply_cond_facts(l, false);
                self.apply_cond_facts(r, false);
            }
            (Expr::Binary(op, l, r), true) => self.apply_true_cmp_facts(*op, l, r),
            (Expr::Binary(op, l, r), false) => self.apply_false_cmp_facts(*op, l, r),
            _ => {}
        }
    }

    fn apply_true_cmp_facts(&mut self, op: BinOp, l: &Expr, r: &Expr) {
        // Any numeric relational/equality check succeeding proves the operand is not `NaN`.
        if matches!(
            op,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::StrictEq
        ) {
            if let Expr::Ident(v) = l {
                if self.vars.get(v) == Some(&NumKind::NonNegI31OrNan)
                    && (exact_f64_lit(r).is_some_and(|n| !n.is_nan())
                        || matches!(r, Expr::Ident(v2) if v2 == v))
                {
                    self.vars.insert(v.clone(), NumKind::NonNegI31);
                }
            }
            if let Expr::Ident(v) = r {
                if self.vars.get(v) == Some(&NumKind::NonNegI31OrNan)
                    && exact_f64_lit(l).is_some_and(|n| !n.is_nan())
                {
                    self.vars.insert(v.clone(), NumKind::NonNegI31);
                }
            }
        }
        match op {
            BinOp::Lt => self.record_lt_str_len(l, r),
            BinOp::Gt => self.record_lt_str_len(r, l),
            BinOp::StrictNe => self.record_ne_zero_str_len(l, r),
            _ => {}
        }
    }

    fn apply_false_cmp_facts(&mut self, op: BinOp, l: &Expr, r: &Expr) {
        match op {
            BinOp::Ge => self.record_lt_str_len(l, r),
            BinOp::Le => self.record_lt_str_len(r, l),
            BinOp::StrictEq => self.record_ne_zero_str_len(l, r),
            _ => {}
        }
    }

    fn record_lt_str_len(&mut self, idx_or_zero: &Expr, len_expr: &Expr) {
        if let Some(s) = self.resolve_str_len(len_expr) {
            if let Expr::Ident(iv) = idx_or_zero {
                if self.is_nonneg_int(iv) {
                    self.in_bounds.insert((iv.clone(), s));
                }
            } else if is_pos_zero(idx_or_zero) {
                self.nonempty_strs.insert(s);
            }
        }
    }

    fn record_ne_zero_str_len(&mut self, l: &Expr, r: &Expr) {
        if let Some(s) = self.resolve_str_len(l) {
            if is_pos_zero(r) {
                self.nonempty_strs.insert(s);
            }
        } else if let Some(s) = self.resolve_str_len(r) {
            if is_pos_zero(l) {
                self.nonempty_strs.insert(s);
            }
        }
    }

    fn join_with(&mut self, other: &Self) {
        let mut next_vars = HashMap::new();
        for (k, &v1) in &self.vars {
            if let Some(&v2) = other.vars.get(k) {
                next_vars.insert(k.clone(), v1.join(v2));
            }
        }
        self.vars = next_vars;
        self.str_lens
            .retain(|k, v| other.str_lens.get(k) == Some(v));
        self.in_bounds.retain(|p| other.in_bounds.contains(p));
        self.nonempty_strs
            .retain(|s| other.nonempty_strs.contains(s));
    }

    /// Merges post-`if` environments into `self` accounting for exiting branches.
    pub(super) fn merge_if_branches(
        &mut self,
        cond: &Expr,
        mut then_env: Self,
        then_exits: bool,
        else_result: Option<(Self, bool)>,
    ) {
        if let Some((else_env, else_exits)) = else_result {
            match (then_exits, else_exits) {
                (true, false) => *self = else_env,
                (false, true) => *self = then_env,
                (false, false) => {
                    then_env.join_with(&else_env);
                    *self = then_env;
                }
                (true, true) => {}
            }
        } else if then_exits {
            self.apply_cond_facts(cond, false);
        } else {
            let mut fallthrough_env = self.clone();
            fallthrough_env.apply_cond_facts(cond, false);
            then_env.join_with(&fallthrough_env);
            *self = then_env;
        }
    }

    /// Computes a sound loop-entry `NumEnv` by invalidating all variables written inside the loop
    /// except those whose every in-loop assignment stays within a provable `NumKind` fixed point
    /// (e.g. counted index `m = m + 1 >>> 0` staying `U32`).
    pub(super) fn prepare_for_loop(&self, cond: Option<&Expr>, body: &[Stmt]) -> Self {
        let mut written = HashSet::new();
        if let Some(c) = cond {
            collect_written_idents_expr(c, &mut written);
        }
        collect_written_idents_stmts(body, &mut written);
        let mut loop_env = self.clone();
        for w in &written {
            loop_env.str_lens.remove(w);
            loop_env.str_lens.retain(|_, s| s != w);
            loop_env.in_bounds.retain(|(i, s)| i != w && s != w);
            loop_env.nonempty_strs.remove(w);
        }

        let mut scan = LoopWriteScan::default();
        if let Some(c) = cond {
            scan.visit_expr(c);
        }
        scan.visit_stmts(body);
        let LoopWriteScan { rhs_map, disq, .. } = scan;

        // Fixed point over written vars that already had a `NumKind` on loop entry.
        for w in &written {
            if disq.contains(w) || !loop_env.vars.contains_key(w) {
                loop_env.vars.remove(w);
            }
        }
        for _ in 0..4 {
            let mut changed = false;
            let keys: Vec<String> = written
                .iter()
                .filter(|w| loop_env.vars.contains_key(*w))
                .cloned()
                .collect();
            for v in keys {
                let mut joined = loop_env.vars[&v];
                let mut ok = true;
                if let Some(rhss) = rhs_map.get(&v) {
                    for rhs in rhss {
                        if let Some(rk) = loop_env.classify_expr(rhs) {
                            joined = joined.join(rk);
                        } else {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    loop_env.vars.remove(&v);
                    changed = true;
                } else if loop_env.vars[&v] != joined {
                    loop_env.vars.insert(v, joined);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        loop_env
    }
}

/// Returns `true` iff `expr` is a pure, side-effect-free integer/charCodeAt expression safe to
/// duplicate or fold from 2 evaluations down to 1.
pub(super) fn is_pure_range_operand(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(_) => true,
        Expr::Unary(UnOp::BitNot, inner) => is_pure_index_expr(inner),
        Expr::Binary(
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr,
            l,
            r,
        ) => is_pure_index_expr(l) && is_pure_index_expr(r),
        Expr::Call(callee, args) => match callee.as_ref() {
            Expr::Member(obj, prop) => {
                if prop == "charCodeAt"
                    && args.len() == 1
                    && matches!(obj.as_ref(), Expr::Ident(_) | Expr::Str(_))
                {
                    return is_pure_index_expr(&args[0]);
                }
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math") {
                    if prop == "clz32" && args.len() == 1 {
                        return is_pure_index_expr(&args[0]);
                    }
                    if prop == "imul" && args.len() == 2 {
                        return is_pure_index_expr(&args[0]) && is_pure_index_expr(&args[1]);
                    }
                }
                false
            }
            _ => false,
        },
        _ => false,
    }
}

// =============================================================================================
// Unsigned Range Check Folding `(x >= A && x <= B)` -> `(x - A >>> 0) <= (B - A)`
// =============================================================================================

#[derive(Clone, Copy, Debug)]
enum RangeBound {
    /// Inclusive lower bound `x >= lo` (or `x < lo` in negated `||` form).
    Lo(i64),
    /// Inclusive upper bound `x <= hi` (or `x > hi` in negated `||` form).
    Hi(i64),
}

fn exact_i64_i32(expr: &Expr) -> Option<i64> {
    exact_i32(expr).map(i64::from)
}

/// Extracts `(operand, RangeBound)` from a single comparison for an inclusive `&&` range check.
/// Tightens strict `<`/`>` by 1 (sound only once the caller proves `operand` is integer or
/// `NonNegI31OrNan`).
fn extract_in_range_bound(expr: &Expr) -> Option<(Expr, RangeBound)> {
    let Expr::Binary(op, l, r) = expr else {
        return None;
    };
    if let Some(c) = exact_i64_i32(r) {
        if is_pure_range_operand(l) {
            return match op {
                BinOp::Ge => Some(((**l).clone(), RangeBound::Lo(c))),
                BinOp::Gt => Some(((**l).clone(), RangeBound::Lo(c + 1))),
                BinOp::Le => Some(((**l).clone(), RangeBound::Hi(c))),
                BinOp::Lt => Some(((**l).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    if let Some(c) = exact_i64_i32(l) {
        if is_pure_range_operand(r) {
            return match op {
                BinOp::Le => Some(((**r).clone(), RangeBound::Lo(c))),
                BinOp::Lt => Some(((**r).clone(), RangeBound::Lo(c + 1))),
                BinOp::Ge => Some(((**r).clone(), RangeBound::Hi(c))),
                BinOp::Gt => Some(((**r).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    None
}

/// Extracts `(operand, RangeBound)` from a single comparison for an exclusive `||` outside-range
/// check (`x < lo || x > hi` -> `(x - lo >>> 0) > (hi - lo)`).
fn extract_out_range_bound(expr: &Expr) -> Option<(Expr, RangeBound)> {
    let Expr::Binary(op, l, r) = expr else {
        return None;
    };
    if let Some(c) = exact_i64_i32(r) {
        if is_pure_range_operand(l) {
            return match op {
                BinOp::Lt => Some(((**l).clone(), RangeBound::Lo(c))),
                BinOp::Le => Some(((**l).clone(), RangeBound::Lo(c + 1))),
                BinOp::Gt => Some(((**l).clone(), RangeBound::Hi(c))),
                BinOp::Ge => Some(((**l).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    if let Some(c) = exact_i64_i32(l) {
        if is_pure_range_operand(r) {
            return match op {
                BinOp::Gt => Some(((**r).clone(), RangeBound::Lo(c))),
                BinOp::Ge => Some(((**r).clone(), RangeBound::Lo(c + 1))),
                BinOp::Lt => Some(((**r).clone(), RangeBound::Hi(c))),
                BinOp::Le => Some(((**r).clone(), RangeBound::Hi(c - 1))),
                _ => None,
            };
        }
    }
    None
}

fn try_fold_range_pair(cmp1: &Expr, cmp2: &Expr, negated: bool, env: &NumEnv) -> Option<Expr> {
    let (x1, b1) = if negated {
        extract_out_range_bound(cmp1)?
    } else {
        extract_in_range_bound(cmp1)?
    };
    let (x2, b2) = if negated {
        extract_out_range_bound(cmp2)?
    } else {
        extract_in_range_bound(cmp2)?
    };
    if x1 != x2 {
        return None;
    }
    let (lo, hi) = match (b1, b2) {
        (RangeBound::Lo(lo), RangeBound::Hi(hi)) | (RangeBound::Hi(hi), RangeBound::Lo(lo)) => {
            (lo, hi)
        }
        _ => return None,
    };
    let kind = env.classify_expr(&x1)?;
    build_folded_range_check(&x1, lo, hi, negated, kind)
}

fn build_folded_range_check(
    x: &Expr,
    lo: i64,
    hi: i64,
    negated: bool,
    kind: NumKind,
) -> Option<Expr> {
    if lo < 0 || hi > i64::from(i32::MAX) {
        return None;
    }
    if lo > hi {
        return if !negated {
            Some(Expr::Bool(false))
        } else if kind.is_exact_int32_or_u32() {
            Some(Expr::Bool(true))
        } else {
            None
        };
    }
    if lo == hi {
        return if !negated && (kind.is_exact_int32_or_u32() || kind == NumKind::NonNegI31OrNan) {
            Some(jsast::binary(
                BinOp::StrictEq,
                x.clone(),
                Expr::Num(lo as f64),
            ))
        } else if negated && kind.is_exact_int32_or_u32() {
            Some(jsast::binary(
                BinOp::StrictNe,
                x.clone(),
                Expr::Num(lo as f64),
            ))
        } else {
            None
        };
    }
    // Now `0 <= lo < hi <= 0x7fff_ffff`.
    let cmp_op = if negated { BinOp::Gt } else { BinOp::Le };
    if lo == 0 {
        if kind.is_nonneg_int() {
            return Some(jsast::binary(cmp_op, x.clone(), Expr::Num(hi as f64)));
        }
        if kind == NumKind::I32 {
            let lhs = jsast::binary(BinOp::UShr, x.clone(), Expr::Num(0.0));
            return Some(jsast::binary(cmp_op, lhs, Expr::Num(hi as f64)));
        }
        // `NonNegI31OrNan` MUST NOT fold at `lo == 0` because `(NaN | 0) = 0 <= hi` is true!
        return None;
    }
    let width = hi - lo;
    if kind.is_exact_int32_or_u32() {
        let sub = jsast::binary(BinOp::Sub, x.clone(), Expr::Num(lo as f64));
        let shifted = jsast::binary(BinOp::UShr, sub, Expr::Num(0.0));
        return Some(jsast::binary(cmp_op, shifted, Expr::Num(width as f64)));
    }
    if kind == NumKind::NonNegI31OrNan && !negated {
        // Sound for ALL `x ∈ {0..=0x7fff_ffff} ∪ {NaN}` when `lo >= 1`:
        // `(NaN | 0) = 0`, `(0 - lo) >>> 0 = 2^32 - lo > hi - lo` -> `false`.
        let coerced = jsast::binary(BinOp::BitOr, x.clone(), Expr::Num(0.0));
        let sub = jsast::binary(BinOp::Sub, coerced, Expr::Num(lo as f64));
        let shifted = jsast::binary(BinOp::UShr, sub, Expr::Num(0.0));
        return Some(jsast::binary(BinOp::Le, shifted, Expr::Num(width as f64)));
    }
    None
}

fn fold_range_logical(expr: &mut Expr, op: BinOp, env: &mut NumEnv) {
    let negated = op == BinOp::Or;
    let replacement = {
        let Expr::Binary(_, l, r) = expr else {
            return;
        };
        fold_range_expr(l, env);
        let mut r_env = env.clone();
        r_env.apply_cond_facts(l, !negated);
        fold_range_expr(r, &mut r_env);
        env.invalidate_written_expr(r);

        // 1. Direct `cmp1 && cmp2` / `cmp1 || cmp2`:
        let mut out = try_fold_range_pair(l, r, negated, env);
        // 2. Left-associated `(P op cmp1) op cmp2`:
        if out.is_none() {
            if let Expr::Binary(lop, p, cmp1) = l.as_ref() {
                if *lop == op {
                    let mut p_env = env.clone();
                    p_env.apply_cond_facts(p, !negated);
                    out = try_fold_range_pair(cmp1, r, negated, &p_env)
                        .map(|folded| jsast::binary(op, (**p).clone(), folded));
                }
            }
        }
        // 3. Right-associated `cmp1 op (cmp2 op Q)`:
        if out.is_none() {
            if let Expr::Binary(rop, cmp2, q) = r.as_ref() {
                if *rop == op {
                    out = try_fold_range_pair(l, cmp2, negated, env)
                        .map(|folded| jsast::binary(op, folded, (**q).clone()));
                }
            }
        }
        out
    };
    if let Some(repl) = replacement {
        *expr = repl;
    }
}

fn fold_range_expr(expr: &mut Expr, env: &mut NumEnv) {
    match expr {
        Expr::Assign(lhs, rhs) => {
            fold_range_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.update_assign(id, rhs);
            } else {
                fold_range_expr(lhs, env);
            }
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            fold_range_expr(lhs, env);
            fold_range_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.invalidate(id);
            }
        }
        Expr::Binary(op @ (BinOp::And | BinOp::Or), _, _) => {
            let op = *op;
            fold_range_logical(expr, op, env);
        }
        Expr::Cond(c, t, e) => {
            fold_range_expr(c, env);
            let mut t_env = env.clone();
            t_env.apply_cond_facts(c, true);
            fold_range_expr(t, &mut t_env);
            let mut e_env = env.clone();
            e_env.apply_cond_facts(c, false);
            fold_range_expr(e, &mut e_env);
            env.invalidate_written_expr(t);
            env.invalidate_written_expr(e);
        }
        Expr::Member(o, _) | Expr::Unary(_, o) => fold_range_expr(o, env),
        Expr::Index(o, i) | Expr::Binary(_, o, i) => {
            fold_range_expr(o, env);
            fold_range_expr(i, env);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            fold_range_expr(c, env);
            for a in args {
                fold_range_expr(a, env);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                fold_range_expr(x, env);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                fold_range_expr(v, env);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => {
            let mut inner = NumEnv::default();
            fold_range_expr(b, &mut inner);
        }
        Expr::Arrow(_, ArrowBody::Block(s)) => {
            let mut inner = NumEnv::default();
            let empty = HashSet::new();
            let empty_rc = HashMap::new();
            fold_range_stmts(s, &mut inner, &empty, &empty_rc);
        }
        _ => {}
    }
}

/// Matches a 1- or 2-statement branch of the form `[dst = rhs]` or `[dst = rhs; break L;]`.
fn match_assign_opt_break(stmts: &[Stmt]) -> Option<(&str, &Expr, Option<&str>)> {
    match stmts {
        [Stmt::ExprStmt(Expr::Assign(lhs, rhs))] => {
            let Expr::Ident(dst) = lhs.as_ref() else {
                return None;
            };
            Some((dst.as_str(), rhs.as_ref(), None))
        }
        [Stmt::ExprStmt(Expr::Assign(lhs, rhs)), Stmt::Break(Some(lbl))] => {
            let Expr::Ident(dst) = lhs.as_ref() else {
                return None;
            };
            Some((dst.as_str(), rhs.as_ref(), Some(lbl.as_str())))
        }
        _ => None,
    }
}

/// Negates a simple relational comparison on a pure operand (e.g. `x < A` -> `x >= A`).
fn negate_rel_cmp(expr: &Expr) -> Option<Expr> {
    let Expr::Binary(op, l, r) = expr else {
        return None;
    };
    let inv = match op {
        BinOp::Lt => BinOp::Ge,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        BinOp::Ge => BinOp::Lt,
        _ => return None,
    };
    Some(jsast::binary(inv, (**l).clone(), (**r).clone()))
}

/// Tries to fold `if (cmp1) { dst = cmp2; [break L;] } else { dst = false; [break L;] }` (and
/// symmetric `true` / swapped-arm forms) into `[dst = folded_range; [break L;]]`.
fn try_fold_range_if_assign(
    cond: &Expr,
    then_b: &[Stmt],
    else_b: &[Stmt],
    env: &NumEnv,
) -> Option<Vec<Stmt>> {
    let (dst1, rhs1, brk1) = match_assign_opt_break(then_b)?;
    let (dst2, rhs2, brk2) = match_assign_opt_break(else_b)?;
    if dst1 != dst2 || brk1 != brk2 {
        return None;
    }
    let folded = match (rhs1, rhs2) {
        // `if (cmp1) { dst = cmp2; } else { dst = false; }` -> `dst = cmp1 && cmp2`
        (cmp2, Expr::Bool(false)) => try_fold_range_pair(cond, cmp2, false, env)?,
        // `if (cmp1) { dst = false; } else { dst = cmp2; }` -> `dst = !cmp1 && cmp2`
        (Expr::Bool(false), cmp2) => {
            let neg_cond = negate_rel_cmp(cond)?;
            try_fold_range_pair(&neg_cond, cmp2, false, env)?
        }
        // `if (cmp1) { dst = true; } else { dst = cmp2; }` -> `dst = cmp1 || cmp2`
        (Expr::Bool(true), cmp2) => try_fold_range_pair(cond, cmp2, true, env)?,
        // `if (cmp1) { dst = cmp2; } else { dst = true; }` -> `dst = !cmp1 || cmp2`
        (cmp2, Expr::Bool(true)) => {
            let neg_cond = negate_rel_cmp(cond)?;
            try_fold_range_pair(&neg_cond, cmp2, true, env)?
        }
        _ => return None,
    };
    let mut out = vec![jsast::assign_stmt(jsast::id(dst1.to_owned()), folded)];
    if let Some(lbl) = brk1 {
        out.push(Stmt::Break(Some(lbl.to_owned())));
    }
    Some(out)
}

fn try_fold_range_stmt_at(
    stmts: &[Stmt],
    i: usize,
    env: &NumEnv,
    local_names: &HashSet<String>,
    read_counts: &HashMap<String, usize>,
) -> Option<(usize, Vec<Stmt>)> {
    // Pattern 1S-b: `tmp = cmp1; if (tmp) { dst = cmp2; [break L;] } else { dst = false; [break L;] }`
    // where `tmp` is a single-read local temp (`jmespath`, `diff`).
    if i + 1 < stmts.len() {
        if let (
            Stmt::ExprStmt(Expr::Assign(lhs, cmp1)),
            Stmt::If(Expr::Ident(cond_id), then_b, Some(else_b)),
        ) = (&stmts[i], &stmts[i + 1])
        {
            if let Expr::Ident(tmp) = lhs.as_ref() {
                if tmp == cond_id
                    && local_names.contains(tmp)
                    && read_counts.get(tmp).copied().unwrap_or(0) == 1
                {
                    if let Some(repl) = try_fold_range_if_assign(cmp1, then_b, else_b, env) {
                        return Some((2, repl));
                    }
                }
            }
        }
    }

    // Pattern 1S-a: `if (cmp1) { dst = cmp2; [break L;] } else { dst = false; [break L;] }`
    if let Stmt::If(cond, then_b, Some(else_b)) = &stmts[i] {
        if let Some(repl) = try_fold_range_if_assign(cond, then_b, else_b, env) {
            return Some((1, repl));
        }
    }
    None
}

fn fold_range_stmts(
    stmts: &mut Vec<Stmt>,
    env: &mut NumEnv,
    local_names: &HashSet<String>,
    read_counts: &HashMap<String, usize>,
) {
    let mut i = 0usize;
    while i < stmts.len() {
        if let Some((consumed, repl)) =
            try_fold_range_stmt_at(stmts, i, env, local_names, read_counts)
        {
            if let Some(Stmt::ExprStmt(Expr::Assign(lhs, rhs))) = repl.first() {
                if let Expr::Ident(dst) = lhs.as_ref() {
                    env.update_assign(dst, rhs);
                }
            }
            let n = repl.len();
            stmts.splice(i..i + consumed, repl);
            i += n;
            continue;
        }

        match &mut stmts[i] {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => fold_range_expr(e, env),
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    fold_range_expr(e, env);
                    env.update_assign(n, e);
                } else {
                    env.invalidate(n);
                }
            }
            Stmt::Const(n, e) => {
                fold_range_expr(e, env);
                env.update_assign(n, e);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        fold_range_expr(e, env);
                        env.update_assign(n, e);
                    } else {
                        env.invalidate(n);
                    }
                }
            }
            Stmt::If(cond, then_b, else_b) => {
                fold_range_expr(cond, env);
                let mut then_env = env.clone();
                then_env.apply_cond_facts(cond, true);
                fold_range_stmts(then_b, &mut then_env, local_names, read_counts);
                let then_exits = stmts_definitely_exit(then_b);
                let else_result = else_b.as_mut().map(|eb| {
                    let mut else_env = env.clone();
                    else_env.apply_cond_facts(cond, false);
                    fold_range_stmts(eb, &mut else_env, local_names, read_counts);
                    let else_exits = stmts_definitely_exit(eb);
                    (else_env, else_exits)
                });
                env.merge_if_branches(cond, then_env, then_exits, else_result);
            }
            Stmt::Switch(disc, cases) => {
                fold_range_expr(disc, env);
                let mut written = HashSet::new();
                for c in cases.iter_mut() {
                    for t in &mut c.tests {
                        let mut te = env.clone();
                        fold_range_expr(t, &mut te);
                    }
                    let mut ce = env.clone();
                    fold_range_stmts(&mut c.body, &mut ce, local_names, read_counts);
                    collect_written_idents_stmts(&c.body, &mut written);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::Block(b) => fold_range_stmts(b, env, local_names, read_counts),
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmt(inner, &mut written);
                let mut ie = env.clone();
                let mut tmp = vec![inner.as_ref().clone()];
                fold_range_stmts(&mut tmp, &mut ie, local_names, read_counts);
                if tmp.len() == 1 {
                    *inner.as_mut() = tmp.pop().unwrap();
                } else {
                    *inner.as_mut() = Stmt::Block(tmp);
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::LoopForever(_, b) => {
                let mut le = env.prepare_for_loop(None, b);
                fold_range_stmts(b, &mut le, local_names, read_counts);
                env.invalidate_written_stmts(b);
            }
            Stmt::While(cond, b) => {
                let mut le = env.prepare_for_loop(Some(cond), b);
                fold_range_expr(cond, &mut le);
                let mut body_env = le.clone();
                body_env.apply_cond_facts(cond, true);
                fold_range_stmts(b, &mut body_env, local_names, read_counts);
                env.invalidate_written_expr(cond);
                env.invalidate_written_stmts(b);
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(body, &mut written);
                collect_written_idents_stmts(handler, &mut written);
                written.insert(binding.clone());
                let mut be = env.clone();
                fold_range_stmts(body, &mut be, local_names, read_counts);
                let mut he = env.clone();
                for w in &written {
                    he.invalidate(w);
                    env.invalidate(w);
                }
                fold_range_stmts(handler, &mut he, local_names, read_counts);
            }
            Stmt::FunctionDecl { body, .. } => {
                let mut inner = NumEnv::default();
                let empty = HashSet::new();
                let empty_rc = HashMap::new();
                fold_range_stmts(body, &mut inner, &empty, &empty_rc);
            }
            _ => {}
        }
        i += 1;
    }
}

pub(super) fn fold_unsigned_range_checks(body: &mut Vec<Stmt>, local_names: &HashSet<String>) {
    let read_counts = tally_ident_reads(body);
    let mut env = NumEnv::default();
    fold_range_stmts(body, &mut env, local_names, &read_counts);
}
