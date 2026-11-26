//! Rewrites `__rt.bstr_sub` / `__rt.bstr_sub_from` / `__rt.bstr_sub_to` calls to direct
//! `s.slice(...)` (or `s` / `""`) whenever BOTH bounds (`0 <= start <= end <= s.length`) AND
//! UTF-8 char boundaries at `start`/`end` are statically proven from dominating guards,
//! `KnownAscii` provenance, or loop-invariant boundary induction.

use std::collections::{BTreeMap, BTreeSet};

use crate::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, Expr, Stmt, SwitchCase, Visit, VisitMut,
};

use super::super::super::js::ast_util::{rt_member, stmts_definitely_exit};
use super::{collect_closure_captured, collect_fn_locals};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum IndexAtom {
    Const(u32),
    Var(String),
    VarPlus(String, u32),
    VarMinus(String, u32),
    StrLen(String),
}

impl IndexAtom {
    fn mentions(&self, var: &str) -> bool {
        match self {
            IndexAtom::Const(_) => false,
            IndexAtom::Var(v)
            | IndexAtom::VarPlus(v, _)
            | IndexAtom::VarMinus(v, _)
            | IndexAtom::StrLen(v) => v == var,
        }
    }

    fn plus_one(&self) -> Option<IndexAtom> {
        match self {
            IndexAtom::Const(k) => k.checked_add(1).map(IndexAtom::Const),
            IndexAtom::Var(v) => Some(IndexAtom::VarPlus(v.clone(), 1)),
            IndexAtom::VarPlus(v, k) => k.checked_add(1).map(|n| IndexAtom::VarPlus(v.clone(), n)),
            IndexAtom::VarMinus(v, 1) => Some(IndexAtom::Var(v.clone())),
            IndexAtom::VarMinus(v, k) if *k > 1 => Some(IndexAtom::VarMinus(v.clone(), k - 1)),
            _ => None,
        }
    }
}

fn intersect_map<K: Ord + Clone, V: Eq + Clone>(
    a: &BTreeMap<K, V>,
    b: &BTreeMap<K, V>,
) -> BTreeMap<K, V> {
    a.iter()
        .filter(|(k, v)| b.get(*k) == Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn intersect_set<T: Ord + Clone>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> BTreeSet<T> {
    a.intersection(b).cloned().collect()
}

#[derive(Clone, Default)]
struct SubEnv {
    str_alias: BTreeMap<String, String>,
    len_alias: BTreeMap<String, String>,
    range_fields: BTreeMap<(String, &'static str), IndexAtom>,
    idx_alias: BTreeMap<String, IndexAtom>,
    valid_boundary: BTreeSet<(String, IndexAtom)>,
    lt_len: BTreeSet<(String, IndexAtom)>,
    le_len: BTreeSet<(String, IndexAtom)>,
    le_pairs: BTreeSet<(IndexAtom, IndexAtom)>,
    lt_pairs: BTreeSet<(IndexAtom, IndexAtom)>,
    gt_zero: BTreeSet<String>,
    ge_zero: BTreeSet<String>,
}

impl SubEnv {
    fn resolve_str_ident(&self, name: &str) -> String {
        let mut cur = name;
        for _ in 0..8 {
            if let Some(next) = self.str_alias.get(cur) {
                if next == cur {
                    break;
                }
                cur = next.as_str();
            } else {
                break;
            }
        }
        cur.to_owned()
    }

    fn resolve_str(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(n) => Some(self.resolve_str_ident(n)),
            _ => None,
        }
    }

    fn record_valid_var_boundary(&mut self, s: &str, v: &str) {
        let atom = IndexAtom::Var(v.to_owned());
        self.valid_boundary.insert((s.to_owned(), atom.clone()));
        self.le_len.insert((s.to_owned(), atom));
    }

    fn restore_invariants(&mut self, invariants: &BTreeSet<(String, String)>) {
        for (s, v) in invariants {
            self.record_valid_var_boundary(s, v);
        }
    }

    fn kill_vars_and_restore(
        &mut self,
        modified: &BTreeSet<String>,
        invariants: &BTreeSet<(String, String)>,
    ) {
        for v in modified {
            self.kill_var(v);
        }
        self.restore_invariants(invariants);
    }

    fn kill_var(&mut self, var: &str) {
        self.str_alias.retain(|k, v| k != var && v != var);
        self.len_alias.retain(|k, v| k != var && v != var);
        self.range_fields
            .retain(|(obj, _), atom| obj != var && !atom.mentions(var));
        self.idx_alias
            .retain(|k, atom| k != var && !atom.mentions(var));
        self.valid_boundary
            .retain(|(s, atom)| s != var && !atom.mentions(var));
        self.lt_len
            .retain(|(s, atom)| s != var && !atom.mentions(var));
        self.le_len
            .retain(|(s, atom)| s != var && !atom.mentions(var));
        self.le_pairs
            .retain(|(a, b)| !a.mentions(var) && !b.mentions(var));
        self.lt_pairs
            .retain(|(a, b)| !a.mentions(var) && !b.mentions(var));
        self.gt_zero.remove(var);
        self.ge_zero.remove(var);
    }

    fn intersect(a: &SubEnv, b: &SubEnv) -> SubEnv {
        SubEnv {
            str_alias: intersect_map(&a.str_alias, &b.str_alias),
            len_alias: intersect_map(&a.len_alias, &b.len_alias),
            range_fields: intersect_map(&a.range_fields, &b.range_fields),
            idx_alias: intersect_map(&a.idx_alias, &b.idx_alias),
            valid_boundary: intersect_set(&a.valid_boundary, &b.valid_boundary),
            lt_len: intersect_set(&a.lt_len, &b.lt_len),
            le_len: intersect_set(&a.le_len, &b.le_len),
            le_pairs: intersect_set(&a.le_pairs, &b.le_pairs),
            lt_pairs: intersect_set(&a.lt_pairs, &b.lt_pairs),
            gt_zero: intersect_set(&a.gt_zero, &b.gt_zero),
            ge_zero: intersect_set(&a.ge_zero, &b.ge_zero),
        }
    }

    fn is_lt_len_var(&self, root_s: &str, var: &str) -> bool {
        let v = IndexAtom::Var(var.to_owned());
        if self.lt_len.contains(&(root_s.to_owned(), v)) {
            return true;
        }
        if let Some(orig) = self.idx_alias.get(var) {
            return self.lt_len.contains(&(root_s.to_owned(), orig.clone()));
        }
        false
    }

    fn resolve_u32_atom(
        &self,
        expr: &Expr,
        root_s_hint: Option<&str>,
        u32_locals: &BTreeSet<String>,
    ) -> Option<IndexAtom> {
        match expr {
            Expr::Num(n)
                if n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n <= f64::from(u32::MAX) =>
            {
                Some(IndexAtom::Const(*n as u32))
            }
            Expr::Ident(n) => {
                if let Some(s) = self.len_alias.get(n) {
                    return Some(IndexAtom::StrLen(s.clone()));
                }
                if u32_locals.contains(n) || self.ge_zero.contains(n) {
                    return Some(IndexAtom::Var(n.clone()));
                }
                None
            }
            Expr::Member(obj, prop) => {
                if prop == "length" {
                    if let Some(s) = self.resolve_str(obj) {
                        return Some(IndexAtom::StrLen(s));
                    }
                }
                if prop == "start" || prop == "end" {
                    if let Expr::Ident(obj_n) = obj.as_ref() {
                        let key = if prop == "start" { "start" } else { "end" };
                        if let Some(atom) = self.range_fields.get(&(obj_n.clone(), key)) {
                            return Some(atom.clone());
                        }
                    }
                }
                None
            }
            Expr::Binary(jsast::BinOp::UShr, l, r) if matches!(r.as_ref(), Expr::Num(n) if *n == 0.0) => {
                self.resolve_u32_atom(l, root_s_hint, u32_locals)
            }
            Expr::Binary(jsast::BinOp::Add, l, r) => {
                let (v, k) = match (l.as_ref(), r.as_ref()) {
                    (Expr::Ident(v), Expr::Num(k)) | (Expr::Num(k), Expr::Ident(v))
                        if k.is_finite()
                            && k.fract() == 0.0
                            && *k >= 1.0
                            && *k <= 65536.0
                            && u32_locals.contains(v) =>
                    {
                        (v, *k as u32)
                    }
                    _ => return None,
                };
                // `v + k` cannot wrap `u32` when `v < s.length` (string length < 2^30).
                let no_wrap = match root_s_hint {
                    Some(s) => self.is_lt_len_var(s, v),
                    None => self
                        .lt_len
                        .iter()
                        .any(|(_, a)| matches!(a, IndexAtom::Var(x) if x == v)),
                };
                no_wrap.then(|| IndexAtom::VarPlus(v.clone(), k))
            }
            Expr::Binary(jsast::BinOp::Sub, l, r) => {
                if let (Expr::Ident(v), Expr::Num(k)) = (l.as_ref(), r.as_ref()) {
                    if *k == 1.0 && u32_locals.contains(v) && self.gt_zero.contains(v) {
                        return Some(IndexAtom::VarMinus(v.clone(), 1));
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn is_valid_boundary_atom(
        &self,
        known_ascii: &BTreeSet<String>,
        root_s: &str,
        atom: &IndexAtom,
    ) -> bool {
        match atom {
            IndexAtom::Const(0) => true,
            IndexAtom::StrLen(s) if s == root_s => true,
            _ => {
                let key = (root_s.to_owned(), atom.clone());
                if self.valid_boundary.contains(&key) {
                    return true;
                }
                if known_ascii.contains(root_s)
                    && (self.le_len.contains(&key) || self.lt_len.contains(&key))
                {
                    return true;
                }
                if let IndexAtom::Var(v) = atom {
                    if let Some(orig) = self.idx_alias.get(v) {
                        if orig != atom {
                            return self.is_valid_boundary_atom(known_ascii, root_s, orig);
                        }
                    }
                }
                false
            }
        }
    }

    fn is_le_atoms(&self, root_s: &str, a: &IndexAtom, b: &IndexAtom) -> bool {
        if a == b || matches!(a, IndexAtom::Const(0)) {
            return true;
        }
        if matches!(b, IndexAtom::StrLen(s) if s == root_s) {
            return true;
        }
        if let (IndexAtom::Const(x), IndexAtom::Const(y)) = (a, b) {
            return x <= y;
        }
        if self.le_pairs.contains(&(a.clone(), b.clone()))
            || self.lt_pairs.contains(&(a.clone(), b.clone()))
        {
            return true;
        }
        // Check `Var(v) <= VarPlus(v, k)` and `VarPlus(v, k1) <= VarPlus(v, k2)` when `v < len`.
        match (a, b) {
            (IndexAtom::Var(v1), IndexAtom::VarPlus(v2, _)) if v1 == v2 => {
                return self.is_lt_len_var(root_s, v1);
            }
            (IndexAtom::VarPlus(v1, k1), IndexAtom::VarPlus(v2, k2)) if v1 == v2 && k1 <= k2 => {
                return self.is_lt_len_var(root_s, v1);
            }
            _ => {}
        }
        // Resolve one step through `idx_alias`.
        if let IndexAtom::Var(vb) = b {
            if let Some(orig_b) = self.idx_alias.get(vb) {
                if orig_b != b && self.is_le_atoms(root_s, a, orig_b) {
                    return true;
                }
            }
        }
        if let IndexAtom::Var(va) = a {
            if let Some(orig_a) = self.idx_alias.get(va) {
                if orig_a != a && self.is_le_atoms(root_s, orig_a, b) {
                    return true;
                }
            }
        }
        false
    }
}

fn is_u32_expr(e: &Expr, u32_locals: &BTreeSet<String>) -> bool {
    match e {
        Expr::Num(n) => n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n <= f64::from(u32::MAX),
        Expr::Ident(n) => u32_locals.contains(n),
        Expr::Member(_, prop) => prop == "length",
        Expr::Binary(jsast::BinOp::UShr, _, r) => matches!(r.as_ref(), Expr::Num(n) if *n == 0.0),
        Expr::Cond(_, t, f) => is_u32_expr(t, u32_locals) && is_u32_expr(f, u32_locals),
        _ => false,
    }
}

struct U32Scan<'a> {
    locals: &'a BTreeSet<String>,
    writes: BTreeMap<String, Vec<Expr>>,
    bad: BTreeSet<String>,
}

impl Visit for U32Scan<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                if self.locals.contains(n) {
                    self.writes.entry(n.clone()).or_default().push(init.clone());
                }
                self.visit_expr(init);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(init) = init {
                        if self.locals.contains(n) {
                            self.writes.entry(n.clone()).or_default().push(init.clone());
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
                self.bad.insert(binding.clone());
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, e: &Expr) {
        match e {
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.locals.contains(n) {
                        self.writes
                            .entry(n.clone())
                            .or_default()
                            .push((**rhs).clone());
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if self.locals.contains(n) {
                        self.bad.insert(n.clone());
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr(self, e),
        }
    }
}

fn collect_u32_locals(
    body: &[Stmt],
    locals: &BTreeSet<String>,
    captured: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut scan = U32Scan {
        locals,
        writes: BTreeMap::new(),
        bad: captured.clone(),
    };
    scan.visit_stmts(body);
    let mut u32_set: BTreeSet<String> = scan
        .writes
        .keys()
        .filter(|k| !scan.bad.contains(*k))
        .cloned()
        .collect();
    loop {
        let remove: Vec<String> = u32_set
            .iter()
            .filter(|k| !scan.writes[*k].iter().all(|e| is_u32_expr(e, &u32_set)))
            .cloned()
            .collect();
        if remove.is_empty() {
            break;
        }
        for k in remove {
            u32_set.remove(&k);
        }
    }
    u32_set
}

struct AssignedVarsCollector<'a>(&'a mut BTreeSet<String>);

impl Visit for AssignedVarsCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, Some(e)) | Stmt::Const(n, e) => {
                self.0.insert(n.clone());
                self.visit_expr(e);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        self.0.insert(n.clone());
                        self.visit_expr(e);
                    }
                }
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                self.0.insert(binding.clone());
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, e: &Expr) {
        match e {
            Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    self.0.insert(n.clone());
                } else if let Expr::Member(obj, _) = lhs.as_ref() {
                    if let Expr::Ident(n) = obj.as_ref() {
                        self.0.insert(n.clone());
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr(self, e),
        }
    }
}

fn collect_assigned_vars_stmts(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    AssignedVarsCollector(out).visit_stmts(stmts);
}

struct BreakFinder<'a> {
    label: &'a str,
    found: bool,
}

impl Visit for BreakFinder<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if self.found {
            return;
        }
        match stmt {
            Stmt::Break(Some(l)) if l == self.label => self.found = true,
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, _: &Expr) {}
}

fn contains_break_to(stmt: &Stmt, label: &str) -> bool {
    let mut finder = BreakFinder {
        label,
        found: false,
    };
    finder.visit_stmt(stmt);
    finder.found
}

fn record_ascii_char_at(env: &mut SubEnv, root_s: String, idx_atom: IndexAtom) {
    env.valid_boundary
        .insert((root_s.clone(), idx_atom.clone()));
    env.lt_len.insert((root_s.clone(), idx_atom.clone()));
    env.le_len.insert((root_s.clone(), idx_atom.clone()));
    if let Some(next) = idx_atom.plus_one() {
        env.valid_boundary.insert((root_s.clone(), next.clone()));
        env.le_len.insert((root_s.clone(), next.clone()));
        env.lt_pairs.insert((idx_atom.clone(), next.clone()));
        env.le_pairs.insert((idx_atom.clone(), next.clone()));
        if let IndexAtom::VarPlus(ref v, _) = idx_atom {
            let v_atom = IndexAtom::Var(v.clone());
            env.lt_pairs.insert((v_atom.clone(), next.clone()));
            env.le_pairs.insert((v_atom, next));
        }
    }
    if let IndexAtom::Var(ref v) = idx_atom {
        if let Some(orig) = env.idx_alias.get(v).cloned() {
            env.valid_boundary.insert((root_s.clone(), orig.clone()));
            env.lt_len.insert((root_s.clone(), orig.clone()));
            env.le_len.insert((root_s, orig));
        }
    }
}

fn negate_cmp_op(op: jsast::BinOp) -> Option<jsast::BinOp> {
    Some(match op {
        jsast::BinOp::Lt => jsast::BinOp::Ge,
        jsast::BinOp::Le => jsast::BinOp::Gt,
        jsast::BinOp::Gt => jsast::BinOp::Le,
        jsast::BinOp::Ge => jsast::BinOp::Lt,
        jsast::BinOp::StrictEq => jsast::BinOp::StrictNe,
        jsast::BinOp::StrictNe => jsast::BinOp::StrictEq,
        _ => return None,
    })
}

fn match_charcode_call<'a>(env: &SubEnv, e: &'a Expr) -> Option<(String, &'a Expr)> {
    let Expr::Call(callee, args) = e else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    let Expr::Member(obj, method) = callee.as_ref() else {
        return None;
    };
    if method != "charCodeAt" {
        return None;
    }
    let root_s = env.resolve_str(obj)?;
    Some((root_s, &args[0]))
}

fn is_ascii_guard_num(op: jsast::BinOp, k: f64) -> bool {
    if !k.is_finite() || k.fract() != 0.0 || k < 0.0 {
        return false;
    }
    match op {
        jsast::BinOp::StrictEq | jsast::BinOp::Le => k <= 127.0,
        jsast::BinOp::Lt => (1.0..=128.0).contains(&k),
        _ => false,
    }
}

fn try_apply_charcode_guard(
    env: &mut SubEnv,
    eff_op: jsast::BinOp,
    l: &Expr,
    r: &Expr,
    u32_locals: &BTreeSet<String>,
) -> bool {
    if let (Some((root_s, idx_e)), Expr::Num(k)) = (match_charcode_call(env, l), r) {
        if is_ascii_guard_num(eff_op, *k) {
            if let Some(atom) = env.resolve_u32_atom(idx_e, Some(&root_s), u32_locals) {
                record_ascii_char_at(env, root_s, atom);
            }
        }
        return true;
    }
    if let (Expr::Num(k), Some((root_s, idx_e))) = (l, match_charcode_call(env, r)) {
        let flipped = match eff_op {
            jsast::BinOp::StrictEq => jsast::BinOp::StrictEq,
            jsast::BinOp::Gt => jsast::BinOp::Lt,
            jsast::BinOp::Ge => jsast::BinOp::Le,
            _ => return true,
        };
        if is_ascii_guard_num(flipped, *k) {
            if let Some(atom) = env.resolve_u32_atom(idx_e, Some(&root_s), u32_locals) {
                record_ascii_char_at(env, root_s, atom);
            }
        }
        return true;
    }
    false
}

fn apply_nonneg_ident_guard(env: &mut SubEnv, eff_op: jsast::BinOp, l: &Expr, r: &Expr) {
    match (eff_op, l, r) {
        (jsast::BinOp::Ge, Expr::Ident(v), Expr::Num(k))
        | (jsast::BinOp::Le, Expr::Num(k), Expr::Ident(v))
            if *k >= 0.0 =>
        {
            env.ge_zero.insert(v.clone());
            if *k > 0.0 {
                env.gt_zero.insert(v.clone());
            }
        }
        (jsast::BinOp::Gt, Expr::Ident(v), Expr::Num(k))
        | (jsast::BinOp::Lt, Expr::Num(k), Expr::Ident(v))
            if *k >= 0.0 =>
        {
            env.ge_zero.insert(v.clone());
            env.gt_zero.insert(v.clone());
        }
        _ => {}
    }
}

fn apply_atom_order_guard(env: &mut SubEnv, eff_op: jsast::BinOp, a: IndexAtom, b: IndexAtom) {
    match eff_op {
        jsast::BinOp::Lt => {
            env.lt_pairs.insert((a.clone(), b.clone()));
            env.le_pairs.insert((a.clone(), b.clone()));
            if let IndexAtom::StrLen(s) = &b {
                env.lt_len.insert((s.clone(), a.clone()));
                env.le_len.insert((s.clone(), a.clone()));
            }
            if a == IndexAtom::Const(0) {
                if let IndexAtom::Var(v) = b {
                    env.gt_zero.insert(v);
                }
            }
        }
        jsast::BinOp::Le => {
            env.le_pairs.insert((a.clone(), b.clone()));
            if let IndexAtom::StrLen(s) = &b {
                env.le_len.insert((s.clone(), a));
            }
        }
        jsast::BinOp::Gt => {
            env.lt_pairs.insert((b.clone(), a.clone()));
            env.le_pairs.insert((b.clone(), a.clone()));
            if let IndexAtom::StrLen(s) = &a {
                env.lt_len.insert((s.clone(), b.clone()));
                env.le_len.insert((s.clone(), b.clone()));
            }
            if b == IndexAtom::Const(0) {
                if let IndexAtom::Var(v) = a {
                    env.gt_zero.insert(v);
                }
            }
        }
        jsast::BinOp::Ge => {
            env.le_pairs.insert((b.clone(), a.clone()));
            if let IndexAtom::StrLen(s) = &a {
                env.le_len.insert((s.clone(), b));
            }
        }
        jsast::BinOp::StrictNe => {
            if b == IndexAtom::Const(0) {
                if let IndexAtom::Var(v) = a {
                    env.gt_zero.insert(v);
                }
            } else if a == IndexAtom::Const(0) {
                if let IndexAtom::Var(v) = b {
                    env.gt_zero.insert(v);
                }
            }
        }
        _ => {}
    }
}

fn apply_guard(env: &mut SubEnv, cond: &Expr, positive: bool, u32_locals: &BTreeSet<String>) {
    match cond {
        Expr::Unary(jsast::UnOp::Not, inner) => {
            apply_guard(env, inner, !positive, u32_locals);
        }
        Expr::Binary(jsast::BinOp::And, l, r) if positive => {
            apply_guard(env, l, true, u32_locals);
            apply_guard(env, r, true, u32_locals);
        }
        Expr::Binary(jsast::BinOp::Or, l, r) if !positive => {
            apply_guard(env, l, false, u32_locals);
            apply_guard(env, r, false, u32_locals);
        }
        Expr::Binary(op, l, r) => {
            let eff_op = if positive {
                *op
            } else if let Some(neg) = negate_cmp_op(*op) {
                neg
            } else {
                return;
            };
            // 1. Check `s.charCodeAt(idx) === K` (K < 128) or `< K` (K <= 128) or `<= K` (K <= 127).
            if try_apply_charcode_guard(env, eff_op, l, r, u32_locals) {
                return;
            }
            // 2. Ordering / nonzero guards between `u32` atoms (seed non-negative guards for
            // parameter identifiers like `i >= 0` or `i > 0`).
            apply_nonneg_ident_guard(env, eff_op, l, r);
            if let (Some(a), Some(b)) = (
                env.resolve_u32_atom(l, None, u32_locals),
                env.resolve_u32_atom(r, None, u32_locals),
            ) {
                apply_atom_order_guard(env, eff_op, a, b);
            }
        }
        _ => {}
    }
}

#[derive(Default)]
struct CarriedFacts {
    valid: Vec<String>,
    lt_len: Vec<String>,
    le_len: Vec<String>,
    le_from: Vec<IndexAtom>,
    lt_from: Vec<IndexAtom>,
    le_to: Vec<IndexAtom>,
    lt_to: Vec<IndexAtom>,
}

fn snapshot_carried_facts(env: &SubEnv, lhs: &str, atom: &IndexAtom) -> CarriedFacts {
    let mut facts = CarriedFacts::default();
    for (s, a) in &env.valid_boundary {
        if a == atom && s != lhs {
            facts.valid.push(s.clone());
        }
    }
    if let IndexAtom::Var(v) = atom {
        if let Some(orig) = env.idx_alias.get(v) {
            for (s, a) in &env.valid_boundary {
                if a == orig && s != lhs {
                    facts.valid.push(s.clone());
                }
            }
        }
    }
    for (s, a) in &env.lt_len {
        if a == atom && s != lhs {
            facts.lt_len.push(s.clone());
        }
    }
    for (s, a) in &env.le_len {
        if a == atom && s != lhs {
            facts.le_len.push(s.clone());
        }
    }
    for (a, b) in &env.le_pairs {
        if b == atom && !a.mentions(lhs) {
            facts.le_from.push(a.clone());
        }
        if a == atom && !b.mentions(lhs) {
            facts.le_to.push(b.clone());
        }
    }
    for (a, b) in &env.lt_pairs {
        if b == atom && !a.mentions(lhs) {
            facts.lt_from.push(a.clone());
        }
        if a == atom && !b.mentions(lhs) {
            facts.lt_to.push(b.clone());
        }
    }
    facts
}

fn apply_carried_facts(env: &mut SubEnv, lhs: &str, facts: CarriedFacts) {
    let lhs_atom = IndexAtom::Var(lhs.to_owned());
    for s in facts.valid {
        env.record_valid_var_boundary(&s, lhs);
    }
    for s in facts.lt_len {
        env.lt_len.insert((s.clone(), lhs_atom.clone()));
        env.le_len.insert((s, lhs_atom.clone()));
    }
    for s in facts.le_len {
        env.le_len.insert((s, lhs_atom.clone()));
    }
    for a in facts.le_from {
        env.le_pairs.insert((a, lhs_atom.clone()));
    }
    for a in facts.lt_from {
        env.lt_pairs.insert((a.clone(), lhs_atom.clone()));
        env.le_pairs.insert((a, lhs_atom.clone()));
    }
    for b in facts.le_to {
        env.le_pairs.insert((lhs_atom.clone(), b));
    }
    for b in facts.lt_to {
        env.lt_pairs.insert((lhs_atom.clone(), b.clone()));
        env.le_pairs.insert((lhs_atom.clone(), b));
    }
}

fn extract_range_fields(
    env: &SubEnv,
    lhs: &str,
    rhs: &Expr,
    u32_locals: &BTreeSet<String>,
) -> Vec<(&'static str, IndexAtom)> {
    let Expr::Object(entries) = rhs else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(k, v)| {
            let key = match k.as_str() {
                "start" => "start",
                "end" => "end",
                _ => return None,
            };
            let atom = env.resolve_u32_atom(v, None, u32_locals)?;
            (!atom.mentions(lhs)).then_some((key, atom))
        })
        .collect()
}

enum WalkMode<'a> {
    Rewrite,
    VerifyInv(&'a mut BTreeSet<(String, String)>),
}

fn update_env_for_assign(
    env: &mut SubEnv,
    lhs: &str,
    rhs: &Expr,
    u32_locals: &BTreeSet<String>,
    known_ascii: &BTreeSet<String>,
    mode: &mut WalkMode<'_>,
) {
    // Compute pre-kill facts about `rhs`.
    let rhs_str_alias = match rhs {
        Expr::Ident(s) if s != lhs => Some(env.resolve_str_ident(s)),
        _ => None,
    };
    let rhs_len_of = match rhs {
        Expr::Member(obj, prop) if prop == "length" => env.resolve_str(obj),
        _ => None,
    };
    let rhs_range_fields = extract_range_fields(env, lhs, rhs, u32_locals);
    let rhs_atom = env.resolve_u32_atom(rhs, None, u32_locals);

    // If verifying loop-invariant boundary candidates, check whether `rhs` preserves `(s, lhs)`.
    if let WalkMode::VerifyInv(inv) = mode {
        inv.retain(|(s, v)| {
            v != lhs
                || rhs_atom
                    .as_ref()
                    .is_some_and(|a| env.is_valid_boundary_atom(known_ascii, s, a))
        });
    }

    let carried = rhs_atom
        .as_ref()
        .map(|atom| snapshot_carried_facts(env, lhs, atom))
        .unwrap_or_default();

    // Kill all old facts involving `lhs`.
    env.kill_var(lhs);

    if let Some(root_s) = rhs_str_alias {
        if root_s != lhs {
            env.str_alias.insert(lhs.to_owned(), root_s);
        }
    }
    if let Some(root_s) = rhs_len_of {
        if root_s != lhs {
            env.len_alias.insert(lhs.to_owned(), root_s.clone());
            env.record_valid_var_boundary(&root_s, lhs);
        }
    }
    for (field, atom) in rhs_range_fields {
        env.range_fields.insert((lhs.to_owned(), field), atom);
    }
    if let Some(atom) = rhs_atom {
        if !atom.mentions(lhs) {
            env.idx_alias.insert(lhs.to_owned(), atom.clone());
        }
        if let IndexAtom::StrLen(s) = &atom {
            if s != lhs {
                env.record_valid_var_boundary(s, lhs);
            }
        }
    }
    apply_carried_facts(env, lhs, carried);
}

fn try_rewrite_bstr_sub_call(
    expr: &mut Expr,
    env: &SubEnv,
    u32_locals: &BTreeSet<String>,
    known_ascii: &BTreeSet<String>,
) {
    let Expr::Call(callee, args) = expr else {
        return;
    };
    let Some(member) = rt_member(callee) else {
        return;
    };
    let replacement = match (member, args.len()) {
        ("bstr_sub", 4 | 5) => try_fold_sub(args, env, u32_locals, known_ascii),
        ("bstr_sub_from", 3 | 4) => try_fold_sub_from(args, env, u32_locals, known_ascii),
        ("bstr_sub_to", 3 | 4) => try_fold_sub_to(args, env, u32_locals, known_ascii),
        _ => None,
    };
    if let Some(new_expr) = replacement {
        *expr = new_expr;
    }
}

fn try_fold_sub(
    args: &[Expr],
    env: &SubEnv,
    u32_locals: &BTreeSet<String>,
    known_ascii: &BTreeSet<String>,
) -> Option<Expr> {
    if !jsast::is_droppable(&args[3]) || args.get(4).is_some_and(|l| !jsast::is_droppable(l)) {
        return None;
    }
    let root_s = env.resolve_str(&args[0])?;
    let a = env.resolve_u32_atom(&args[1], Some(&root_s), u32_locals)?;
    let b = env.resolve_u32_atom(&args[2], Some(&root_s), u32_locals)?;
    if a == IndexAtom::Const(0)
        && b == IndexAtom::Const(0)
        && jsast::is_droppable(&args[0])
        && jsast::is_droppable(&args[1])
        && jsast::is_droppable(&args[2])
    {
        return Some(jsast::string(""));
    }
    if a == IndexAtom::Const(0)
        && b == IndexAtom::StrLen(root_s.clone())
        && jsast::is_droppable(&args[1])
        && jsast::is_droppable(&args[2])
    {
        return Some(args[0].clone());
    }
    if env.is_valid_boundary_atom(known_ascii, &root_s, &a)
        && env.is_valid_boundary_atom(known_ascii, &root_s, &b)
        && env.is_le_atoms(&root_s, &a, &b)
    {
        return Some(jsast::method_call(
            args[0].clone(),
            "slice",
            vec![args[1].clone(), args[2].clone()],
        ));
    }
    None
}

fn try_fold_sub_from(
    args: &[Expr],
    env: &SubEnv,
    u32_locals: &BTreeSet<String>,
    known_ascii: &BTreeSet<String>,
) -> Option<Expr> {
    if !jsast::is_droppable(&args[2]) || args.get(3).is_some_and(|l| !jsast::is_droppable(l)) {
        return None;
    }
    let root_s = env.resolve_str(&args[0])?;
    let a = env.resolve_u32_atom(&args[1], Some(&root_s), u32_locals)?;
    if a == IndexAtom::Const(0) && jsast::is_droppable(&args[1]) {
        return Some(args[0].clone());
    }
    if env.is_valid_boundary_atom(known_ascii, &root_s, &a) {
        return Some(jsast::method_call(
            args[0].clone(),
            "slice",
            vec![args[1].clone()],
        ));
    }
    None
}

fn try_fold_sub_to(
    args: &[Expr],
    env: &SubEnv,
    u32_locals: &BTreeSet<String>,
    known_ascii: &BTreeSet<String>,
) -> Option<Expr> {
    if !jsast::is_droppable(&args[2]) || args.get(3).is_some_and(|l| !jsast::is_droppable(l)) {
        return None;
    }
    let root_s = env.resolve_str(&args[0])?;
    let b = env.resolve_u32_atom(&args[1], Some(&root_s), u32_locals)?;
    if b == IndexAtom::Const(0) && jsast::is_droppable(&args[0]) && jsast::is_droppable(&args[1]) {
        return Some(jsast::string(""));
    }
    if b == IndexAtom::StrLen(root_s.clone()) && jsast::is_droppable(&args[1]) {
        return Some(args[0].clone());
    }
    if env.is_valid_boundary_atom(known_ascii, &root_s, &b) {
        return Some(jsast::method_call(
            args[0].clone(),
            "slice",
            vec![jsast::num(0), args[1].clone()],
        ));
    }
    None
}

struct CandidateStrsCollector<'a> {
    env: &'a SubEnv,
    out: &'a mut BTreeSet<String>,
}

impl Visit for CandidateStrsCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, e: &Expr) {
        match e {
            Expr::Member(obj, prop) if prop == "length" => {
                if let Some(root_s) = self.env.resolve_str(obj) {
                    self.out.insert(root_s);
                }
                self.visit_expr(obj);
            }
            Expr::Call(callee, args) => {
                if let Expr::Member(obj, method) = callee.as_ref() {
                    if method == "charCodeAt" {
                        if let Some(root_s) = self.env.resolve_str(obj) {
                            self.out.insert(root_s);
                        }
                    }
                }
                if matches!(
                    rt_member(callee),
                    Some("bstr_sub" | "bstr_sub_from" | "bstr_sub_to")
                ) {
                    if let Some(first) = args.first() {
                        if let Some(root_s) = self.env.resolve_str(first) {
                            self.out.insert(root_s);
                        }
                    }
                }
                walk_expr(self, e);
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr(self, e),
        }
    }
}

fn collect_candidate_strs_stmts(stmts: &[Stmt], env: &SubEnv, out: &mut BTreeSet<String>) {
    CandidateStrsCollector { env, out }.visit_stmts(stmts);
}

struct ExprWalk<'a, 'b, 'c> {
    w: &'a SubWalker<'b>,
    env: &'a mut SubEnv,
    mode: &'a mut WalkMode<'c>,
}

impl VisitMut for ExprWalk<'_, '_, '_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Assign(lhs, rhs) => {
                self.visit_expr_mut(rhs);
                if let Expr::Ident(n) = lhs.as_ref() {
                    update_env_for_assign(
                        self.env,
                        n,
                        rhs,
                        self.w.u32_locals,
                        self.w.known_ascii,
                        self.mode,
                    );
                } else {
                    self.visit_expr_mut(lhs);
                    if let Expr::Member(obj, _) = lhs.as_ref() {
                        if let Expr::Ident(n) = obj.as_ref() {
                            self.env.range_fields.retain(|(o, _), _| o != n);
                        }
                    }
                }
                return;
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                self.visit_expr_mut(rhs);
                self.visit_expr_mut(lhs);
                if let Expr::Ident(n) = lhs.as_ref() {
                    if let WalkMode::VerifyInv(inv) = self.mode {
                        inv.retain(|(_, v)| v != n);
                    }
                    self.env.kill_var(n);
                }
                return;
            }
            Expr::Binary(jsast::BinOp::And, l, r) => {
                self.visit_expr_mut(l);
                let mut r_env = self.env.clone();
                apply_guard(&mut r_env, l, true, self.w.u32_locals);
                self.w.walk_expr(r, &mut r_env, self.mode);
                return;
            }
            Expr::Cond(c, t, f) => {
                self.visit_expr_mut(c);
                let mut t_env = self.env.clone();
                apply_guard(&mut t_env, c, true, self.w.u32_locals);
                self.w.walk_expr(t, &mut t_env, self.mode);
                let mut f_env = self.env.clone();
                apply_guard(&mut f_env, c, false, self.w.u32_locals);
                self.w.walk_expr(f, &mut f_env, self.mode);
                *self.env = SubEnv::intersect(&t_env, &f_env);
                return;
            }
            Expr::Arrow(_, _) => return,
            _ => walk_expr_mut(self, expr),
        }
        if matches!(self.mode, WalkMode::Rewrite) {
            try_rewrite_bstr_sub_call(expr, self.env, self.w.u32_locals, self.w.known_ascii);
        }
    }
}

struct SubWalker<'a> {
    u32_locals: &'a BTreeSet<String>,
    known_ascii: &'a BTreeSet<String>,
    captured: &'a BTreeSet<String>,
}

impl SubWalker<'_> {
    fn walk_expr(&self, expr: &mut Expr, env: &mut SubEnv, mode: &mut WalkMode<'_>) {
        ExprWalk { w: self, env, mode }.visit_expr_mut(expr);
    }

    fn seed_loop_invariants(
        &self,
        loop_body: &[Stmt],
        env: &SubEnv,
        modified: &BTreeSet<String>,
    ) -> BTreeSet<(String, String)> {
        let mut inv: BTreeSet<(String, String)> = BTreeSet::new();
        for (s, atom) in &env.valid_boundary {
            if let IndexAtom::Var(v) = atom {
                if !modified.contains(s)
                    && !self.captured.contains(s)
                    && !self.captured.contains(v)
                    && modified.contains(v)
                {
                    inv.insert((s.clone(), v.clone()));
                }
            }
        }
        let mut candidate_strs: BTreeSet<String> = self.known_ascii.clone();
        for s in env.len_alias.values().chain(env.str_alias.values()) {
            candidate_strs.insert(s.clone());
        }
        for (s, _) in &env.valid_boundary {
            candidate_strs.insert(s.clone());
        }
        collect_candidate_strs_stmts(loop_body, env, &mut candidate_strs);
        for (v, atom) in &env.idx_alias {
            if *atom == IndexAtom::Const(0) && modified.contains(v) && !self.captured.contains(v) {
                for s in &candidate_strs {
                    if !modified.contains(s) && !self.captured.contains(s) {
                        inv.insert((s.clone(), v.clone()));
                    }
                }
            }
        }
        inv
    }

    fn walk_loop(
        &self,
        test: Option<&mut Expr>,
        loop_body: &mut [Stmt],
        env: &mut SubEnv,
        active_inv: &BTreeSet<(String, String)>,
        mode: &mut WalkMode<'_>,
    ) {
        let mut modified = BTreeSet::new();
        collect_assigned_vars_stmts(loop_body, &mut modified);
        let mut inv = self.seed_loop_invariants(loop_body, env, &modified);

        for _ in 0..4 {
            let prev_len = inv.len();
            let mut check_env = env.clone();
            for v in &modified {
                check_env.kill_var(v);
            }
            let mut merged_inv = active_inv.clone();
            for (s, v) in &inv {
                if !modified.contains(s) {
                    merged_inv.insert((s.clone(), v.clone()));
                    check_env.record_valid_var_boundary(s, v);
                }
            }
            if let Some(t) = test.as_deref() {
                apply_guard(&mut check_env, t, true, self.u32_locals);
            }
            let mut verify = WalkMode::VerifyInv(&mut inv);
            self.walk_stmts(loop_body, &mut check_env, &merged_inv, &mut verify);
            if inv.len() == prev_len {
                break;
            }
        }

        if let WalkMode::VerifyInv(outer) = mode {
            outer.retain(|(s, v)| !modified.contains(v) || inv.contains(&(s.clone(), v.clone())));
        }

        let mut loop_env = env.clone();
        loop_env.kill_vars_and_restore(&modified, &inv);
        let mut merged_inv = active_inv.clone();
        merged_inv.extend(inv.iter().cloned());

        let mut body_env = if let Some(t) = test {
            self.walk_expr(t, &mut loop_env, mode);
            let mut b = loop_env.clone();
            apply_guard(&mut b, t, true, self.u32_locals);
            b
        } else {
            loop_env.clone()
        };
        self.walk_stmts(loop_body, &mut body_env, &merged_inv, mode);
        *env = loop_env;
    }

    fn walk_if(
        &self,
        test: &mut Expr,
        then_b: &mut [Stmt],
        else_b: Option<&mut [Stmt]>,
        env: &mut SubEnv,
        active_inv: &BTreeSet<(String, String)>,
        mode: &mut WalkMode<'_>,
    ) {
        self.walk_expr(test, env, mode);
        let mut then_env = env.clone();
        apply_guard(&mut then_env, test, true, self.u32_locals);
        self.walk_stmts(then_b, &mut then_env, active_inv, mode);
        let mut else_env = env.clone();
        apply_guard(&mut else_env, test, false, self.u32_locals);
        let else_exits = if let Some(else_stmts) = else_b {
            self.walk_stmts(else_stmts, &mut else_env, active_inv, mode);
            stmts_definitely_exit(else_stmts)
        } else {
            false
        };
        let then_exits = stmts_definitely_exit(then_b);
        if then_exits && !else_exits {
            *env = else_env;
        } else if else_exits && !then_exits {
            *env = then_env;
        } else {
            *env = SubEnv::intersect(&then_env, &else_env);
        }
    }

    fn walk_switch(
        &self,
        disc: &mut Expr,
        cases: &mut [SwitchCase],
        env: &mut SubEnv,
        active_inv: &BTreeSet<(String, String)>,
        mode: &mut WalkMode<'_>,
    ) {
        self.walk_expr(disc, env, mode);
        let mut modified = BTreeSet::new();
        for c in cases.iter() {
            collect_assigned_vars_stmts(&c.body, &mut modified);
        }
        for c in cases {
            for t in &mut c.tests {
                self.walk_expr(t, env, mode);
            }
            let mut case_env = env.clone();
            self.walk_stmts(&mut c.body, &mut case_env, active_inv, mode);
        }
        env.kill_vars_and_restore(&modified, active_inv);
    }

    fn walk_labeled(
        &self,
        label: &str,
        inner: &mut Stmt,
        env: &mut SubEnv,
        active_inv: &BTreeSet<(String, String)>,
        mode: &mut WalkMode<'_>,
    ) {
        let is_loop = matches!(inner, Stmt::LoopForever(..) | Stmt::While(..));
        if is_loop {
            self.walk_stmts(std::slice::from_mut(inner), env, active_inv, mode);
            return;
        }
        let pre = env.clone();
        let has_break = contains_break_to(inner, label);
        let mut modified = BTreeSet::new();
        if has_break {
            collect_assigned_vars_stmts(std::slice::from_ref(&*inner), &mut modified);
        }
        self.walk_stmts(std::slice::from_mut(inner), env, active_inv, mode);
        if has_break {
            let mut safe_pre = pre;
            safe_pre.kill_vars_and_restore(&modified, active_inv);
            *env = SubEnv::intersect(env, &safe_pre);
        }
    }

    fn walk_try_catch(
        &self,
        try_body: &mut [Stmt],
        binding: &str,
        handler: &mut [Stmt],
        env: &mut SubEnv,
        active_inv: &BTreeSet<(String, String)>,
        mode: &mut WalkMode<'_>,
    ) {
        let mut try_env = env.clone();
        let mut modified = BTreeSet::new();
        collect_assigned_vars_stmts(try_body, &mut modified);
        modified.insert(binding.to_owned());
        self.walk_stmts(try_body, &mut try_env, active_inv, mode);
        let mut handler_env = env.clone();
        handler_env.kill_vars_and_restore(&modified, active_inv);
        self.walk_stmts(handler, &mut handler_env, active_inv, mode);
        if stmts_definitely_exit(handler) {
            *env = try_env;
        } else {
            *env = SubEnv::intersect(&try_env, &handler_env);
        }
    }

    fn walk_stmts(
        &self,
        stmts: &mut [Stmt],
        env: &mut SubEnv,
        active_inv: &BTreeSet<(String, String)>,
        mode: &mut WalkMode<'_>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                    self.walk_expr(init, env, mode);
                    update_env_for_assign(env, n, init, self.u32_locals, self.known_ascii, mode);
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        if let Some(init) = init {
                            self.walk_expr(init, env, mode);
                            update_env_for_assign(
                                env,
                                n,
                                init,
                                self.u32_locals,
                                self.known_ascii,
                                mode,
                            );
                        }
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    self.walk_expr(e, env, mode);
                }
                Stmt::If(test, then_b, else_b) => {
                    self.walk_if(test, then_b, else_b.as_deref_mut(), env, active_inv, mode);
                }
                Stmt::Switch(disc, cases) => {
                    self.walk_switch(disc, cases, env, active_inv, mode);
                }
                Stmt::Block(b) => self.walk_stmts(b, env, active_inv, mode),
                Stmt::Labeled(label, inner) => {
                    self.walk_labeled(label, inner, env, active_inv, mode);
                }
                Stmt::LoopForever(_, loop_body) => {
                    self.walk_loop(None, loop_body, env, active_inv, mode);
                }
                Stmt::While(test, loop_body) => {
                    self.walk_loop(Some(test), loop_body, env, active_inv, mode);
                }
                Stmt::TryCatch {
                    body: try_body,
                    binding,
                    handler,
                } => {
                    self.walk_try_catch(try_body, binding, handler, env, active_inv, mode);
                }
                _ => {}
            }
        }
    }
}

/// Rewrites `__rt.bstr_sub` / `__rt.bstr_sub_from` / `__rt.bstr_sub_to` calls to direct
/// `s.slice(...)` (or `s` / `""`) whenever BOTH bounds (`0 <= start <= end <= s.length`) AND
/// UTF-8 char boundaries at `start`/`end` are statically proven from dominating guards,
/// `KnownAscii` provenance, or loop-invariant boundary induction.
pub(in crate::lower::bstr) fn elide_proven_bstr_sub(
    body: &mut [Stmt],
    known_ascii: &BTreeSet<String>,
) {
    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    let mut captured = BTreeSet::new();
    collect_closure_captured(body, &locals, &mut captured);
    let u32_locals = collect_u32_locals(body, &locals, &captured);
    let mut env = SubEnv::default();
    let empty_inv = BTreeSet::new();
    let mut mode = WalkMode::Rewrite;
    SubWalker {
        u32_locals: &u32_locals,
        known_ascii,
        captured: &captured,
    }
    .walk_stmts(body, &mut env, &empty_inv, &mut mode);
}
