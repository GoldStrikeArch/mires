//! Dense ASCII delimiter membership lowering to 128/256-byte `Uint8Array` LUTs and chained
//! string / integer equality `if / else if` & guard chain lowering to `switch (x)`.

use std::collections::HashSet;

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp, VisitMut};

use super::super::ast_util::{
    collect_written_idents_stmt, collect_written_idents_stmts, exact_i32, exact_u8,
    has_unlabeled_switch_break, stmts_definitely_exit,
};
use super::fold::num_expr_i32;
use super::guards::{is_pure_range_operand, NumEnv};

// =============================================================================================
// Dense ASCII Delimiter Membership -> 128/256-Byte `Uint8Array` LUT
// =============================================================================================

const MIN_LUT_DISTINCT_BYTES: usize = 6;

fn build_u8_lut_test(disc: Expr, bytes: &[u8], negated: bool) -> Expr {
    let max_b = bytes.iter().copied().max().unwrap_or(0);
    let len = if max_b <= 127 { 128usize } else { 256usize };
    let mut table = vec![Expr::Num(0.0); len];
    for &b in bytes {
        table[b as usize] = Expr::Num(1.0);
    }
    let lut = Expr::New(Box::new(jsast::id("Uint8Array")), vec![Expr::Array(table)]);
    let load = jsast::index(lut, disc);
    jsast::binary(
        if negated {
            BinOp::StrictNe
        } else {
            BinOp::StrictEq
        },
        load,
        Expr::Num(1.0),
    )
}

/// Extracts `(operand, byte)` from `c === k` / `k === c` (when `expect_eq == true`) or
/// `c !== k` / `k !== c` / `!(c === k)` (when `expect_eq == false`).
fn extract_byte_eq_term(expr: &Expr, expect_eq: bool) -> Option<(Expr, u8)> {
    match expr {
        Expr::Unary(UnOp::Not, inner) if !expect_eq => extract_byte_eq_term(inner, true),
        Expr::Binary(op, l, r) => {
            let ok = if expect_eq {
                *op == BinOp::StrictEq
            } else {
                *op == BinOp::StrictNe
            };
            if !ok {
                return None;
            }
            if let Some(b) = exact_u8(r) {
                if is_pure_range_operand(l) {
                    return Some(((**l).clone(), b));
                }
            }
            if let Some(b) = exact_u8(l) {
                if is_pure_range_operand(r) {
                    return Some(((**r).clone(), b));
                }
            }
            None
        }
        _ => None,
    }
}

fn flatten_logical(expr: &Expr, op: BinOp, out: &mut Vec<Expr>) {
    if let Expr::Binary(eop, l, r) = expr {
        if *eop == op {
            flatten_logical(l, op, out);
            flatten_logical(r, op, out);
            return;
        }
    }
    out.push(expr.clone());
}

fn try_fold_byte_lut_chain(expr: &Expr, op: BinOp, env: &NumEnv) -> Option<Expr> {
    let expect_eq = op == BinOp::Or;
    let mut terms = Vec::new();
    flatten_logical(expr, op, &mut terms);
    if terms.len() < MIN_LUT_DISTINCT_BYTES {
        return None;
    }
    let mut out_terms: Vec<Expr> = Vec::with_capacity(terms.len());
    let mut changed = false;
    let mut i = 0usize;
    while i < terms.len() {
        let Some((cand_x, first_b)) = extract_byte_eq_term(&terms[i], expect_eq) else {
            out_terms.push(terms[i].clone());
            i += 1;
            continue;
        };
        // CRITICAL SOUNDNESS: `lut["92"]` is `1` in JS whereas `"92" === 92` is `false`!
        // Only lower when `cand_x` is provably a JS number (never a string/object).
        if env.classify_expr(&cand_x).is_none() {
            out_terms.push(terms[i].clone());
            i += 1;
            continue;
        }
        let mut bytes: Vec<u8> = vec![first_b];
        let mut distinct: HashSet<u8> = HashSet::new();
        distinct.insert(first_b);
        let mut j = i + 1;
        while j < terms.len() {
            if let Some((x2, b2)) = extract_byte_eq_term(&terms[j], expect_eq) {
                if x2 == cand_x {
                    bytes.push(b2);
                    distinct.insert(b2);
                    j += 1;
                    continue;
                }
            }
            break;
        }
        if distinct.len() >= MIN_LUT_DISTINCT_BYTES {
            out_terms.push(build_u8_lut_test(cand_x, &bytes, !expect_eq));
            changed = true;
        } else {
            for t in &terms[i..j] {
                out_terms.push(t.clone());
            }
        }
        i = j;
    }
    if !changed {
        return None;
    }
    let mut iter = out_terms.into_iter();
    let first = iter.next()?;
    Some(iter.fold(first, |acc, next| jsast::binary(op, acc, next)))
}

fn lower_lut_expr(expr: &mut Expr, env: &mut NumEnv) {
    match expr {
        Expr::Assign(lhs, rhs) => {
            lower_lut_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.update_assign(id, rhs);
            } else {
                lower_lut_expr(lhs, env);
            }
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            lower_lut_expr(lhs, env);
            lower_lut_expr(rhs, env);
            if let Expr::Ident(id) = lhs.as_ref() {
                env.invalidate(id);
            }
        }
        Expr::Binary(op @ (BinOp::Or | BinOp::And), _, _) => {
            let op = *op;
            if let Some(folded) = try_fold_byte_lut_chain(expr, op, env) {
                *expr = folded;
                return;
            }
            if let Expr::Binary(_, l, r) = expr {
                lower_lut_expr(l, env);
                let mut r_env = env.clone();
                r_env.apply_cond_facts(l, op == BinOp::And);
                lower_lut_expr(r, &mut r_env);
                env.invalidate_written_expr(r);
            }
        }
        Expr::Cond(c, t, e) => {
            lower_lut_expr(c, env);
            let mut t_env = env.clone();
            t_env.apply_cond_facts(c, true);
            lower_lut_expr(t, &mut t_env);
            let mut e_env = env.clone();
            e_env.apply_cond_facts(c, false);
            lower_lut_expr(e, &mut e_env);
            env.invalidate_written_expr(t);
            env.invalidate_written_expr(e);
        }
        Expr::Member(o, _) | Expr::Unary(_, o) => lower_lut_expr(o, env),
        Expr::Index(o, i) | Expr::Binary(_, o, i) => {
            lower_lut_expr(o, env);
            lower_lut_expr(i, env);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            lower_lut_expr(c, env);
            for a in args {
                lower_lut_expr(a, env);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                lower_lut_expr(x, env);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                lower_lut_expr(v, env);
            }
        }
        Expr::Arrow(_, ArrowBody::Expr(b)) => {
            let mut inner = NumEnv::default();
            lower_lut_expr(b, &mut inner);
        }
        Expr::Arrow(_, ArrowBody::Block(s)) => {
            let mut inner = NumEnv::default();
            lower_lut_stmts(s, &mut inner);
        }
        _ => {}
    }
}

/// Attempts to lower a 2-arm (or 1-arm) dense ASCII `switch (disc)` with `>= 6` distinct byte
/// constants in `0..=255` on a proven-numeric `disc` into a `Uint8Array` LUT check.
fn try_lower_ascii_switch_to_lut(
    disc: &Expr,
    cases: &[jsast::SwitchCase],
    env: &NumEnv,
) -> Option<Stmt> {
    if env.classify_expr(disc).is_none() || cases.is_empty() {
        return None;
    }

    // Group consecutive empty-body non-default `SwitchCase`s into combined arms.
    let mut arms: Vec<(bool, Vec<Expr>, Vec<Stmt>)> = Vec::new();
    let mut pending_tests: Vec<Expr> = Vec::new();
    for (idx, c) in cases.iter().enumerate() {
        let is_default = c.tests.is_empty();
        if is_default {
            if !pending_tests.is_empty() {
                // Non-default case falling straight into `default:` — bail out.
                return None;
            }
            arms.push((true, Vec::new(), c.body.clone()));
        } else {
            pending_tests.extend(c.tests.iter().cloned());
            let is_last = idx + 1 == cases.len();
            if !c.body.is_empty() || is_last {
                arms.push((false, std::mem::take(&mut pending_tests), c.body.clone()));
            }
        }
    }
    if !pending_tests.is_empty() || arms.is_empty() || arms.len() > 2 {
        return None;
    }

    let (match_tests, mut match_body, mut default_body) = match arms.len() {
        1 => {
            let (is_def, tests, body) = arms.pop().unwrap();
            if is_def {
                return None;
            }
            (tests, body, Vec::new())
        }
        2 => {
            let arm1 = arms.pop().unwrap();
            let arm0 = arms.pop().unwrap();
            match (arm0.0, arm1.0) {
                (false, true) => {
                    // Matching arm comes before `default:`: ensure it CANNOT fall through!
                    let exits = matches!(arm0.2.last(), Some(Stmt::Break(None)))
                        || stmts_definitely_exit(&arm0.2);
                    if !exits {
                        return None;
                    }
                    (arm0.1, arm0.2, arm1.2)
                }
                (true, false) => {
                    // `default:` comes before matching arm: ensure `default:` CANNOT fall through!
                    let exits = matches!(arm0.2.last(), Some(Stmt::Break(None)))
                        || stmts_definitely_exit(&arm0.2);
                    if !exits {
                        return None;
                    }
                    (arm1.1, arm1.2, arm0.2)
                }
                _ => return None,
            }
        }
        _ => return None,
    };

    // Every case label must be an exact byte in `0..=255`, with `>= 6` distinct bytes.
    let mut bytes: Vec<u8> = Vec::with_capacity(match_tests.len());
    let mut distinct: HashSet<u8> = HashSet::new();
    for t in &match_tests {
        let b = exact_u8(t)?;
        bytes.push(b);
        distinct.insert(b);
    }
    if distinct.len() < MIN_LUT_DISTINCT_BYTES {
        return None;
    }

    // Strip a single trailing unlabeled `break;` from each arm and reject any interior unlabeled
    // `break;` (which targeted this `switch` and would otherwise target an enclosing loop/switch!).
    if matches!(match_body.last(), Some(Stmt::Break(None))) {
        match_body.pop();
    }
    if matches!(default_body.last(), Some(Stmt::Break(None))) {
        default_body.pop();
    }
    if has_unlabeled_switch_break(&match_body) || has_unlabeled_switch_break(&default_body) {
        return None;
    }

    // Cleanest lowering for boolean assignment: `case ...: dst = true; break; default: dst = false;`
    if let ([Stmt::ExprStmt(Expr::Assign(l1, r1))], [Stmt::ExprStmt(Expr::Assign(l2, r2))]) =
        (match_body.as_slice(), default_body.as_slice())
    {
        if let (Expr::Ident(d1), Expr::Ident(d2)) = (l1.as_ref(), l2.as_ref()) {
            if d1 == d2 {
                match (r1.as_ref(), r2.as_ref()) {
                    (Expr::Bool(true), Expr::Bool(false)) => {
                        return Some(jsast::assign_stmt(
                            jsast::id(d1.clone()),
                            build_u8_lut_test(disc.clone(), &bytes, false),
                        ));
                    }
                    (Expr::Bool(false), Expr::Bool(true)) => {
                        return Some(jsast::assign_stmt(
                            jsast::id(d1.clone()),
                            build_u8_lut_test(disc.clone(), &bytes, true),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    if match_body.is_empty() && default_body.is_empty() {
        return None;
    }
    if match_body.is_empty() {
        return Some(Stmt::If(
            build_u8_lut_test(disc.clone(), &bytes, true),
            default_body,
            None,
        ));
    }
    if default_body.is_empty() {
        return Some(Stmt::If(
            build_u8_lut_test(disc.clone(), &bytes, false),
            match_body,
            None,
        ));
    }
    Some(Stmt::If(
        build_u8_lut_test(disc.clone(), &bytes, false),
        match_body,
        Some(default_body),
    ))
}

fn lower_lut_stmts(stmts: &mut [Stmt], env: &mut NumEnv) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => lower_lut_expr(e, env),
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    lower_lut_expr(e, env);
                    env.update_assign(n, e);
                } else {
                    env.invalidate(n);
                }
            }
            Stmt::Const(n, e) => {
                lower_lut_expr(e, env);
                env.update_assign(n, e);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        lower_lut_expr(e, env);
                        env.update_assign(n, e);
                    } else {
                        env.invalidate(n);
                    }
                }
            }
            Stmt::If(cond, then_b, else_b) => {
                lower_lut_expr(cond, env);
                let mut then_env = env.clone();
                then_env.apply_cond_facts(cond, true);
                lower_lut_stmts(then_b, &mut then_env);
                let then_exits = stmts_definitely_exit(then_b);
                let else_result = else_b.as_mut().map(|eb| {
                    let mut else_env = env.clone();
                    else_env.apply_cond_facts(cond, false);
                    lower_lut_stmts(eb, &mut else_env);
                    let else_exits = stmts_definitely_exit(eb);
                    (else_env, else_exits)
                });
                env.merge_if_branches(cond, then_env, then_exits, else_result);
            }
            Stmt::Switch(disc, cases) => {
                lower_lut_expr(disc, env);
                let mut written = HashSet::new();
                for c in cases.iter_mut() {
                    for t in &mut c.tests {
                        let mut te = env.clone();
                        lower_lut_expr(t, &mut te);
                    }
                    let mut ce = env.clone();
                    lower_lut_stmts(&mut c.body, &mut ce);
                    collect_written_idents_stmts(&c.body, &mut written);
                }
                if let Some(rewritten) = try_lower_ascii_switch_to_lut(disc, cases, env) {
                    *stmt = rewritten;
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::Block(b) => lower_lut_stmts(b, env),
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmt(inner, &mut written);
                let mut ie = env.clone();
                let mut tmp = vec![inner.as_ref().clone()];
                lower_lut_stmts(&mut tmp, &mut ie);
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
                lower_lut_stmts(b, &mut le);
                env.invalidate_written_stmts(b);
            }
            Stmt::While(cond, b) => {
                let mut le = env.prepare_for_loop(Some(cond), b);
                lower_lut_expr(cond, &mut le);
                let mut body_env = le.clone();
                body_env.apply_cond_facts(cond, true);
                lower_lut_stmts(b, &mut body_env);
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
                lower_lut_stmts(body, &mut be);
                let mut he = env.clone();
                for w in &written {
                    he.invalidate(w);
                    env.invalidate(w);
                }
                lower_lut_stmts(handler, &mut he);
            }
            Stmt::FunctionDecl { body, .. } => {
                let mut inner = NumEnv::default();
                lower_lut_stmts(body, &mut inner);
            }
            _ => {}
        }
    }
}

pub(super) fn lower_dense_ascii_luts(body: &mut [Stmt]) {
    let mut env = NumEnv::default();
    lower_lut_stmts(body, &mut env);
}

// =============================================================================================
// Chained String / Integer Equality `if / else if` & Guard Chains -> `switch (x)`
// =============================================================================================

const MIN_CHAIN_SWITCH_ARMS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SwitchKey {
    Str(String),
    Int(i32),
}

impl SwitchKey {
    fn is_str(&self) -> bool {
        matches!(self, Self::Str(_))
    }

    fn to_expr(&self) -> Expr {
        match self {
            Self::Str(s) => Expr::Str(s.clone()),
            Self::Int(n) => num_expr_i32(*n),
        }
    }
}

fn extract_switch_key_lit(expr: &Expr) -> Option<SwitchKey> {
    match expr {
        Expr::Str(s) => Some(SwitchKey::Str(s.clone())),
        _ => exact_i32(expr).map(SwitchKey::Int),
    }
}

/// Extracts `(ident, is_positive, keys)` from a strict-equality disjunction
/// (`x === "A" || x === "B"`, `is_positive = true`) or negated conjunction
/// (`!(x === "A") && !(x === "B")` / `x !== "A" && x !== "B"`, `is_positive = false`).
fn extract_eq_keys(cond: &Expr) -> Option<(String, bool, Vec<SwitchKey>)> {
    match cond {
        Expr::Unary(UnOp::Not, inner) => {
            let (id, pos, keys) = extract_eq_keys(inner)?;
            Some((id, !pos, keys))
        }
        Expr::Binary(op @ (BinOp::StrictEq | BinOp::StrictNe), l, r) => {
            let is_pos = *op == BinOp::StrictEq;
            if let (Expr::Ident(id), Some(k)) = (l.as_ref(), extract_switch_key_lit(r)) {
                return Some((id.clone(), is_pos, vec![k]));
            }
            if let (Some(k), Expr::Ident(id)) = (extract_switch_key_lit(l), r.as_ref()) {
                return Some((id.clone(), is_pos, vec![k]));
            }
            None
        }
        Expr::Binary(BinOp::Or, l, r) => {
            let (id1, pos1, mut k1) = extract_eq_keys(l)?;
            let (id2, pos2, k2) = extract_eq_keys(r)?;
            if pos1 && pos2 && id1 == id2 && k1[0].is_str() == k2[0].is_str() {
                k1.extend(k2);
                Some((id1, true, k1))
            } else {
                None
            }
        }
        Expr::Binary(BinOp::And, l, r) => {
            let (id1, pos1, mut k1) = extract_eq_keys(l)?;
            let (id2, pos2, k2) = extract_eq_keys(r)?;
            if !pos1 && !pos2 && id1 == id2 && k1[0].is_str() == k2[0].is_str() {
                k1.extend(k2);
                Some((id1, false, k1))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Recursively collects switch arms dispatching on `scrut` from `slice`.
///
/// When `consume_all == true` (inside an `else` or negated-`then` sub-slice), any unmatched tail
/// becomes the `default:` body. When `consume_all == false` (at an outer statement list), returns
/// the prefix count `consumed` of statements absorbed into the `switch`.
fn collect_switch_arms(
    slice: &[Stmt],
    scrut: &str,
    key_is_str: bool,
    consume_all: bool,
    arms: &mut Vec<(Vec<SwitchKey>, Vec<Stmt>)>,
    seen: &mut HashSet<SwitchKey>,
) -> Option<(usize, Option<Vec<Stmt>>)> {
    if slice.is_empty() {
        return Some((0, None));
    }
    if let Stmt::If(cond, then_b, else_opt) = &slice[0] {
        if let Some((id, is_pos, raw_keys)) = extract_eq_keys(cond) {
            if id == scrut && raw_keys[0].is_str() == key_is_str {
                // Deduplicate within a single arm and reject any key already matched earlier.
                let mut dedup_keys = Vec::with_capacity(raw_keys.len());
                let mut local_seen = HashSet::new();
                for k in raw_keys {
                    if seen.contains(&k) {
                        return None;
                    }
                    if local_seen.insert(k.clone()) {
                        dedup_keys.push(k);
                    }
                }
                for k in &dedup_keys {
                    seen.insert(k.clone());
                }

                match (is_pos, else_opt) {
                    // Subcase A: `if (x === "A" || ...) { then_b } else { else_b }`
                    (true, Some(else_b)) if !consume_all || slice.len() == 1 => {
                        arms.push((dedup_keys, then_b.clone()));
                        let (_, default) =
                            collect_switch_arms(else_b, scrut, key_is_str, true, arms, seen)?;
                        return Some((1, default));
                    }
                    // Subcase B: `if (x === "A" || ...) { then_b }` (no `else`)
                    (true, None) => {
                        if stmts_definitely_exit(then_b) {
                            arms.push((dedup_keys, then_b.clone()));
                            let (rest_consumed, default) = collect_switch_arms(
                                &slice[1..],
                                scrut,
                                key_is_str,
                                consume_all,
                                arms,
                                seen,
                            )?;
                            return Some((1 + rest_consumed, default));
                        } else if !consume_all || slice.len() == 1 {
                            arms.push((dedup_keys, then_b.clone()));
                            return Some((1, None));
                        }
                    }
                    // Subcase C: `if (!(x === "A") && ...) { then_b } else { else_b }`
                    (false, Some(else_b)) if !consume_all || slice.len() == 1 => {
                        arms.push((dedup_keys, else_b.clone()));
                        let (_, default) =
                            collect_switch_arms(then_b, scrut, key_is_str, true, arms, seen)?;
                        return Some((1, default));
                    }
                    // Subcase D: `if (!(x === "A") && ...) { then_b } tail...` where `then_b`
                    // definitely exits (`jmespath$eval`, `jmespath$functions::call`, `projection`).
                    (false, None) if slice.len() > 1 && stmts_definitely_exit(then_b) => {
                        arms.push((dedup_keys, slice[1..].to_vec()));
                        let (_, default) =
                            collect_switch_arms(then_b, scrut, key_is_str, true, arms, seen)?;
                        return Some((slice.len(), default));
                    }
                    _ => {}
                }
                // Key matched `scrut` structurally, but shape wasn't convertible; undo `seen`
                // and treat `slice` as unmatched tail if `consume_all`.
                for k in &local_seen {
                    seen.remove(k);
                }
            }
        }
    }
    if consume_all {
        Some((slice.len(), Some(slice.to_vec())))
    } else {
        Some((0, None))
    }
}

fn try_build_chained_switch(slice: &[Stmt]) -> Option<(usize, Stmt)> {
    let Stmt::If(first_cond, _, _) = slice.first()? else {
        return None;
    };
    let (scrut, _, first_keys) = extract_eq_keys(first_cond)?;
    let key_is_str = first_keys[0].is_str();

    let mut arms: Vec<(Vec<SwitchKey>, Vec<Stmt>)> = Vec::new();
    let mut seen: HashSet<SwitchKey> = HashSet::new();
    let (consumed, default_body) =
        collect_switch_arms(slice, &scrut, key_is_str, false, &mut arms, &mut seen)?;
    if consumed == 0 || arms.len() < MIN_CHAIN_SWITCH_ARMS {
        return None;
    }

    // CRITICAL SOUNDNESS: NEVER wrap any branch body containing an unlabeled `break;` into a
    // `switch`, because the new `switch` would capture the `break;` intended for an outer loop
    // or outer `switch`!
    for (_, body) in &arms {
        if has_unlabeled_switch_break(body) {
            return None;
        }
    }
    if let Some(ref def) = default_body {
        if has_unlabeled_switch_break(def) {
            return None;
        }
    }

    let has_default = default_body.as_ref().is_some_and(|d| !d.is_empty());
    let total_arms = arms.len();
    let mut cases: Vec<jsast::SwitchCase> =
        Vec::with_capacity(total_arms + usize::from(has_default));
    for (idx, (keys, mut body)) in arms.into_iter().enumerate() {
        let is_last_case = !has_default && idx + 1 == total_arms;
        if !is_last_case && !stmts_definitely_exit(&body) {
            body.push(Stmt::Break(None));
        }
        cases.push(jsast::SwitchCase {
            tests: keys.into_iter().map(|k| k.to_expr()).collect(),
            body,
        });
    }
    if let Some(def) = default_body {
        if !def.is_empty() {
            cases.push(jsast::SwitchCase {
                tests: Vec::new(),
                body: def,
            });
        }
    }

    Some((consumed, Stmt::Switch(jsast::id(scrut), cases)))
}

struct ChainedSwitchLowerer;

impl VisitMut for ChainedSwitchLowerer {
    fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
        let mut i = 0usize;
        while i < stmts.len() {
            // Top-down: first convert the outermost matching chain at `stmts[i..]`, then recurse
            // into the resulting `switch` arms (or child blocks of any non-matching statement).
            if let Some((consumed, switch_stmt)) = try_build_chained_switch(&stmts[i..]) {
                stmts.splice(i..i + consumed, std::iter::once(switch_stmt));
            }
            self.visit_stmt_mut(&mut stmts[i]);
            i += 1;
        }
    }

    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if let Stmt::Labeled(_, inner) = stmt {
            // Handle `lbl: if (x === "A") ...` directly while preserving the label `lbl:`.
            let mut tmp = vec![inner.as_ref().clone()];
            self.visit_block_mut(&mut tmp);
            if tmp.len() == 1 {
                *inner.as_mut() = tmp.pop().unwrap();
            } else {
                *inner.as_mut() = Stmt::Block(tmp);
            }
            return;
        }
        jsast::walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, _expr: &mut Expr) {}
}

pub(super) fn lower_chained_eq_to_switch(stmts: &mut Vec<Stmt>) {
    ChainedSwitchLowerer.visit_block_mut(stmts);
}
