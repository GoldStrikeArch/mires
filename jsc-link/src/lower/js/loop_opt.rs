//! Compute-loop AST optimizations for `ReleaseSpeed`:
//!
//! 1. [`fold_guarded_f2i`]: collapses nested `if (a) { if (b) { ... } }` and replaces guarded
//!    `f2i` `i32`-clamp ternaries inside `[-2147483648, 2147483647]` with `x | 0` (`earcut`).
//! 2. [`elide_dead_fresh_slice_clones`]: deletes redundant `.slice()` clones on single-use fresh
//!    array literals (`compress256` / `Array::into_iter`).
//! 3. [`unroll_small_const_loops`]: unrolls pure small constant-trip loops (`2..=10` iters) so
//!    index arithmetic, `%`, `Math.imul`, and bounds checks constant-fold away (`sha3::permute`).
//! 4. [`eliminate_dominated_guards`]: deletes redundant dominated panic guards (`if (!H)`,
//!    `if (bi)`, `if (!(x < len))`) and constant-bounded checks (bitmask, constant-table loads).
//! 5. [`hoist_loop_invariant_props`]: LICM of invariant `.buf`/`.off`/`.len` property loads across
//!    cold diverging `core$panicking$*` calls (`fflate/dflt`, `earcut/refine`, `earcut_linked`).
//! 6. [`version_checked_loops`]: hot/cold loop-versioned BCE emitting a single exact pre-loop
//!    invariant guard branching to an unchecked fast loop with panic-identical checked fallback
//!    (`compress512`, `compress256`, `fflate/dflt`, `earcut/refine`).

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp};

use super::post_opt::{alloc_fresh_local, exact_i32, exact_u32, stmts_definitely_diverge};

// =================================================================================================
// 1. Guarded `f2i` Folding & Nested `if` Collapse
// =================================================================================================

pub(super) fn fold_guarded_f2i(stmts: &mut [Stmt]) {
    collapse_nested_ifs(stmts);
    let mut in_i32_range: HashSet<String> = HashSet::new();
    fold_f2i_in_stmts(stmts, &mut in_i32_range);
}

fn collapse_nested_ifs(stmts: &mut [Stmt]) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If(cond, then_body, else_body) => {
                collapse_nested_ifs(then_body);
                if let Some(eb) = else_body {
                    collapse_nested_ifs(eb);
                }
                while else_body.is_none() && then_body.len() == 1 {
                    let can_merge = matches!(then_body.first(), Some(Stmt::If(_, _, None)));
                    if !can_merge {
                        break;
                    }
                    let Some(Stmt::If(inner_cond, inner_then, None)) = then_body.pop() else {
                        break;
                    };
                    let outer = std::mem::replace(cond, Expr::Bool(false));
                    *cond = Expr::Binary(BinOp::And, Box::new(outer), Box::new(inner_cond));
                    *then_body = inner_then;
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    collapse_nested_ifs(&mut c.body);
                }
            }
            Stmt::LoopForever(_, b) | Stmt::While(_, b) | Stmt::Block(b) => {
                collapse_nested_ifs(b);
            }
            Stmt::Labeled(_, inner) => {
                collapse_nested_ifs(std::slice::from_mut(inner.as_mut()));
            }
            Stmt::TryCatch { body, handler, .. } => {
                collapse_nested_ifs(body);
                collapse_nested_ifs(handler);
            }
            _ => {}
        }
    }
}

/// Checks if `cond` (conjunction of comparisons) proves `v >= -2147483648 && v <= 2147483647`.
fn extract_i32_range_vars(cond: &Expr) -> Vec<String> {
    let mut ge_min: HashSet<String> = HashSet::new();
    let mut le_max: HashSet<String> = HashSet::new();
    fn visit(e: &Expr, ge_min: &mut HashSet<String>, le_max: &mut HashSet<String>) {
        match e {
            Expr::Binary(BinOp::And, l, r) => {
                visit(l, ge_min, le_max);
                visit(r, ge_min, le_max);
            }
            Expr::Binary(BinOp::Ge, l, r) => {
                if let (Expr::Ident(v), Some(-2147483648)) = (l.as_ref(), exact_i32(r)) {
                    ge_min.insert(v.clone());
                } else if let (Some(2147483647), Expr::Ident(v)) = (exact_i32(l), r.as_ref()) {
                    le_max.insert(v.clone());
                }
            }
            Expr::Binary(BinOp::Le, l, r) => {
                if let (Expr::Ident(v), Some(2147483647)) = (l.as_ref(), exact_i32(r)) {
                    le_max.insert(v.clone());
                } else if let (Some(-2147483648), Expr::Ident(v)) = (exact_i32(l), r.as_ref()) {
                    ge_min.insert(v.clone());
                }
            }
            _ => {}
        }
    }
    visit(cond, &mut ge_min, &mut le_max);
    ge_min.intersection(&le_max).cloned().collect()
}

/// Matches `v <= -2147483648 ? -2147483648 : (v >= 2147483647 ? 2147483647 : v | 0)` for `v` in
/// `in_i32_range` and replaces it with `v | 0`.
fn is_i32_clamp_for(expr: &Expr, in_i32_range: &HashSet<String>) -> Option<String> {
    let Expr::Cond(c1, t1, e1) = expr else {
        return None;
    };
    let Expr::Binary(BinOp::Le, c1_l, c1_r) = c1.as_ref() else {
        return None;
    };
    let Expr::Ident(v) = c1_l.as_ref() else {
        return None;
    };
    if !in_i32_range.contains(v)
        || exact_i32(c1_r) != Some(-2147483648)
        || exact_i32(t1) != Some(-2147483648)
    {
        return None;
    }
    let Expr::Cond(c2, t2, e2) = e1.as_ref() else {
        return None;
    };
    let Expr::Binary(BinOp::Ge, c2_l, c2_r) = c2.as_ref() else {
        return None;
    };
    if !matches!(c2_l.as_ref(), Expr::Ident(v2) if v2 == v)
        || exact_i32(c2_r) != Some(2147483647)
        || exact_i32(t2) != Some(2147483647)
    {
        return None;
    }
    let Expr::Binary(BinOp::BitOr, e2_l, e2_r) = e2.as_ref() else {
        return None;
    };
    if matches!(e2_l.as_ref(), Expr::Ident(v3) if v3 == v) && exact_i32(e2_r) == Some(0) {
        Some(v.clone())
    } else {
        None
    }
}

fn rewrite_f2i_expr(expr: &mut Expr, in_i32_range: &mut HashSet<String>) {
    if let Some(v) = is_i32_clamp_for(expr, in_i32_range) {
        *expr = Expr::Binary(
            BinOp::BitOr,
            Box::new(Expr::Ident(v)),
            Box::new(Expr::Num(0.0)),
        );
        return;
    }
    match expr {
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            rewrite_f2i_expr(rhs, in_i32_range);
            if let Expr::Ident(n) = lhs.as_ref() {
                in_i32_range.remove(n);
            } else {
                rewrite_f2i_expr(lhs, in_i32_range);
            }
        }
        Expr::Unary(_, e) => rewrite_f2i_expr(e, in_i32_range),
        Expr::Binary(_, l, r) => {
            rewrite_f2i_expr(l, in_i32_range);
            rewrite_f2i_expr(r, in_i32_range);
        }
        Expr::Cond(c, t, e) => {
            rewrite_f2i_expr(c, in_i32_range);
            let mut t_set = in_i32_range.clone();
            for v in extract_i32_range_vars(c) {
                t_set.insert(v);
            }
            rewrite_f2i_expr(t, &mut t_set);
            let mut e_set = in_i32_range.clone();
            rewrite_f2i_expr(e, &mut e_set);
            // Any variable written in either branch is invalidated.
            let mut written = HashSet::new();
            collect_written_idents_expr(t, &mut written);
            collect_written_idents_expr(e, &mut written);
            for w in written {
                in_i32_range.remove(&w);
            }
        }
        Expr::Member(o, _) => rewrite_f2i_expr(o, in_i32_range),
        Expr::Index(o, i) => {
            rewrite_f2i_expr(o, in_i32_range);
            rewrite_f2i_expr(i, in_i32_range);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            rewrite_f2i_expr(c, in_i32_range);
            for a in args {
                rewrite_f2i_expr(a, in_i32_range);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                rewrite_f2i_expr(x, in_i32_range);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                rewrite_f2i_expr(v, in_i32_range);
            }
        }
        _ => {}
    }
}

fn fold_f2i_in_stmts(stmts: &mut [Stmt], in_i32_range: &mut HashSet<String>) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                rewrite_f2i_expr(e, in_i32_range);
            }
            Stmt::Let(n, init) => {
                if let Some(e) = init {
                    rewrite_f2i_expr(e, in_i32_range);
                }
                in_i32_range.remove(n);
            }
            Stmt::Const(n, e) => {
                rewrite_f2i_expr(e, in_i32_range);
                in_i32_range.remove(n);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        rewrite_f2i_expr(e, in_i32_range);
                    }
                    in_i32_range.remove(n);
                }
            }
            Stmt::If(cond, then_body, else_body) => {
                rewrite_f2i_expr(cond, in_i32_range);
                let mut then_set = in_i32_range.clone();
                for v in extract_i32_range_vars(cond) {
                    then_set.insert(v);
                }
                fold_f2i_in_stmts(then_body, &mut then_set);
                if let Some(eb) = else_body {
                    let mut else_set = in_i32_range.clone();
                    fold_f2i_in_stmts(eb, &mut else_set);
                }
                let mut written = HashSet::new();
                collect_written_idents_stmts(then_body, &mut written, false);
                if let Some(eb) = else_body {
                    collect_written_idents_stmts(eb, &mut written, false);
                }
                for w in written {
                    in_i32_range.remove(&w);
                }
            }
            Stmt::Switch(disc, cases) => {
                rewrite_f2i_expr(disc, in_i32_range);
                let mut written = HashSet::new();
                for c in cases.iter_mut() {
                    let mut c_set = in_i32_range.clone();
                    fold_f2i_in_stmts(&mut c.body, &mut c_set);
                    collect_written_idents_stmts(&c.body, &mut written, false);
                }
                for w in written {
                    in_i32_range.remove(&w);
                }
            }
            Stmt::LoopForever(_, b) => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(b, &mut written, false);
                for w in &written {
                    in_i32_range.remove(w);
                }
                let mut b_set = in_i32_range.clone();
                fold_f2i_in_stmts(b, &mut b_set);
            }
            Stmt::While(cond, b) => {
                let mut written = HashSet::new();
                collect_written_idents_expr(cond, &mut written);
                collect_written_idents_stmts(b, &mut written, false);
                for w in &written {
                    in_i32_range.remove(w);
                }
                rewrite_f2i_expr(cond, in_i32_range);
                let mut b_set = in_i32_range.clone();
                for v in extract_i32_range_vars(cond) {
                    b_set.insert(v);
                }
                fold_f2i_in_stmts(b, &mut b_set);
            }
            Stmt::Block(b) => fold_f2i_in_stmts(b, in_i32_range),
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(
                    std::slice::from_ref(inner.as_ref()),
                    &mut written,
                    false,
                );
                let mut inner_set = in_i32_range.clone();
                fold_f2i_in_stmts(std::slice::from_mut(inner.as_mut()), &mut inner_set);
                for w in written {
                    in_i32_range.remove(&w);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(body, &mut written, false);
                collect_written_idents_stmts(handler, &mut written, false);
                let mut b_set = in_i32_range.clone();
                fold_f2i_in_stmts(body, &mut b_set);
                let mut h_set = in_i32_range.clone();
                for w in &written {
                    h_set.remove(w);
                    in_i32_range.remove(w);
                }
                fold_f2i_in_stmts(handler, &mut h_set);
            }
            _ => {}
        }
    }
}

// =================================================================================================
// 2. Dead Fresh `.slice()` Clone Elision (`aC = [...]; Z = aC.slice(); data: Z.slice()`)
// =================================================================================================

pub(super) fn elide_dead_fresh_slice_clones(stmts: &mut [Stmt], local_names: &HashSet<String>) {
    // Run up to 3 rounds so chained `aC = [...]; Z = aC.slice(); data: Z.slice()` collapses fully.
    for _ in 0..3 {
        let mut reads: HashMap<String, usize> = HashMap::new();
        let mut writes: HashMap<String, usize> = HashMap::new();
        count_rw_stmts(stmts, &mut reads, &mut writes);
        if !elide_fresh_slice_in_block(stmts, local_names, &reads, &writes) {
            break;
        }
    }
}

fn fresh_array_obj_keys(expr: &Expr) -> Option<Vec<String>> {
    let Expr::Array(items) = expr else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    let mut expected: Option<Vec<String>> = None;
    for item in items {
        let Expr::Object(entries) = item else {
            return None;
        };
        let mut keys: Vec<String> = entries.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        keys.dedup();
        if keys.len() != entries.len() {
            return None;
        }
        match &expected {
            None => expected = Some(keys),
            Some(prev) if prev == &keys => {}
            Some(_) => return None,
        }
    }
    expected
}

fn is_void_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Undefined)
        || matches!(expr, Expr::Unary(UnOp::Void, inner) if exact_u32(inner) == Some(0))
}

fn is_shallow_obj_copy_of(expr: &Expr, param: &str, expected_keys: &[String]) -> bool {
    let Expr::Object(entries) = expr else {
        return false;
    };
    if entries.len() != expected_keys.len() {
        return false;
    }
    let mut keys: Vec<String> = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        let ok = matches!(
            v,
            Expr::Member(obj, prop) if prop == k && matches!(obj.as_ref(), Expr::Ident(id) if id == param)
        );
        if !ok {
            return false;
        }
        keys.push(k.clone());
    }
    keys.sort();
    keys == expected_keys
}

fn is_shallow_obj_clone_map(expr: &Expr, target: &str, expected_keys: &[String]) -> bool {
    let Expr::Call(callee, args) = expr else {
        return false;
    };
    if args.len() != 1 {
        return false;
    }
    let is_target_map = matches!(
        callee.as_ref(),
        Expr::Member(obj, prop) if prop == "map" && matches!(obj.as_ref(), Expr::Ident(id) if id == target)
    );
    if !is_target_map {
        return false;
    }
    let Expr::Arrow(params, ArrowBody::Expr(body)) = &args[0] else {
        return false;
    };
    if params.len() != 1 {
        return false;
    }
    let p = &params[0];
    if is_shallow_obj_copy_of(body, p, expected_keys) {
        return true;
    }
    if let Expr::Cond(cond, then_e, else_e) = body.as_ref() {
        let is_p_ne_undef = |e: &Expr| -> Option<bool> {
            let Expr::Binary(op, l, r) = e else {
                return None;
            };
            let matches_pair = (matches!(l.as_ref(), Expr::Ident(id) if id == p)
                && is_void_expr(r))
                || (is_void_expr(l) && matches!(r.as_ref(), Expr::Ident(id) if id == p));
            if !matches_pair {
                return None;
            }
            match op {
                BinOp::StrictNe => Some(true),
                BinOp::StrictEq => Some(false),
                _ => None,
            }
        };
        if let Some(true) = is_p_ne_undef(cond) {
            return is_shallow_obj_copy_of(then_e, p, expected_keys) && is_void_expr(else_e);
        }
        if let Some(false) = is_p_ne_undef(cond) {
            return is_void_expr(then_e) && is_shallow_obj_copy_of(else_e, p, expected_keys);
        }
    }
    false
}

fn is_fresh_array_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Array(_) => true,
        Expr::Call(callee, args) if args.is_empty() => {
            matches!(callee.as_ref(), Expr::Member(_, prop) if prop == "slice")
        }
        _ => false,
    }
}

fn extract_fresh_array_def(
    stmt: &Stmt,
    fresh_vars: &HashSet<String>,
    fresh_obj_keys: &HashMap<String, Vec<String>>,
) -> Option<(String, Option<Vec<String>>)> {
    fn from_expr(
        expr: &Expr,
        fresh_vars: &HashSet<String>,
        fresh_obj_keys: &HashMap<String, Vec<String>>,
    ) -> Option<(String, Option<Vec<String>>)> {
        match expr {
            Expr::Assign(lhs, rhs) => {
                let Expr::Ident(name) = lhs.as_ref() else {
                    return None;
                };
                if let Some(keys) = fresh_array_obj_keys(rhs) {
                    return Some((name.clone(), Some(keys)));
                }
                if let Expr::Ident(src) = rhs.as_ref() {
                    if fresh_vars.contains(src) {
                        return Some((name.clone(), fresh_obj_keys.get(src).cloned()));
                    }
                }
                if is_fresh_array_expr(rhs) {
                    return Some((name.clone(), None));
                }
                None
            }
            Expr::Seq(items) => items
                .last()
                .and_then(|e| from_expr(e, fresh_vars, fresh_obj_keys)),
            _ => None,
        }
    }
    match stmt {
        Stmt::ExprStmt(e) => from_expr(e, fresh_vars, fresh_obj_keys),
        _ => None,
    }
}

fn replace_ident_slice_call_expr(
    expr: &mut Expr,
    target: &str,
    expected_obj_keys: Option<&[String]>,
) -> bool {
    if let Expr::Call(callee, args) = expr {
        if args.is_empty() {
            if let Expr::Member(obj, prop) = callee.as_ref() {
                if prop == "slice" && matches!(obj.as_ref(), Expr::Ident(id) if id == target) {
                    *expr = Expr::Ident(target.to_owned());
                    return true;
                }
            }
        }
    }
    if let Some(keys) = expected_obj_keys {
        if is_shallow_obj_clone_map(expr, target, keys) {
            *expr = Expr::Ident(target.to_owned());
            return true;
        }
    }
    match expr {
        Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) => {
            replace_ident_slice_call_expr(l, target, expected_obj_keys)
                | replace_ident_slice_call_expr(r, target, expected_obj_keys)
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => {
            replace_ident_slice_call_expr(e, target, expected_obj_keys)
        }
        Expr::Index(o, i) => {
            replace_ident_slice_call_expr(o, target, expected_obj_keys)
                | replace_ident_slice_call_expr(i, target, expected_obj_keys)
        }
        Expr::Cond(c, t, e) => {
            replace_ident_slice_call_expr(c, target, expected_obj_keys)
                | replace_ident_slice_call_expr(t, target, expected_obj_keys)
                | replace_ident_slice_call_expr(e, target, expected_obj_keys)
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            let mut ch = replace_ident_slice_call_expr(c, target, expected_obj_keys);
            for a in args {
                ch |= replace_ident_slice_call_expr(a, target, expected_obj_keys);
            }
            ch
        }
        Expr::Array(items) | Expr::Seq(items) => {
            let mut ch = false;
            for x in items {
                ch |= replace_ident_slice_call_expr(x, target, expected_obj_keys);
            }
            ch
        }
        Expr::Object(entries) => {
            let mut ch = false;
            for (_, v) in entries {
                ch |= replace_ident_slice_call_expr(v, target, expected_obj_keys);
            }
            ch
        }
        _ => false,
    }
}

fn replace_ident_slice_call_stmt(
    stmt: &mut Stmt,
    target: &str,
    expected_obj_keys: Option<&[String]>,
) -> bool {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
            replace_ident_slice_call_expr(e, target, expected_obj_keys)
        }
        Stmt::Let(_, Some(e)) => replace_ident_slice_call_expr(e, target, expected_obj_keys),
        Stmt::Lets(decls) => {
            let mut ch = false;
            for (_, init) in decls {
                if let Some(e) = init {
                    ch |= replace_ident_slice_call_expr(e, target, expected_obj_keys);
                }
            }
            ch
        }
        Stmt::If(c, a, b) => {
            let mut ch = replace_ident_slice_call_expr(c, target, expected_obj_keys);
            for s in a {
                ch |= replace_ident_slice_call_stmt(s, target, expected_obj_keys);
            }
            if let Some(b) = b {
                for s in b {
                    ch |= replace_ident_slice_call_stmt(s, target, expected_obj_keys);
                }
            }
            ch
        }
        Stmt::Switch(d, cases) => {
            let mut ch = replace_ident_slice_call_expr(d, target, expected_obj_keys);
            for c in cases {
                for s in &mut c.body {
                    ch |= replace_ident_slice_call_stmt(s, target, expected_obj_keys);
                }
            }
            ch
        }
        Stmt::Block(b) | Stmt::LoopForever(_, b) => {
            let mut ch = false;
            for s in b {
                ch |= replace_ident_slice_call_stmt(s, target, expected_obj_keys);
            }
            ch
        }
        Stmt::While(c, b) => {
            let mut ch = replace_ident_slice_call_expr(c, target, expected_obj_keys);
            for s in b {
                ch |= replace_ident_slice_call_stmt(s, target, expected_obj_keys);
            }
            ch
        }
        Stmt::Labeled(_, inner) => replace_ident_slice_call_stmt(inner, target, expected_obj_keys),
        Stmt::TryCatch { body, handler, .. } => {
            let mut ch = false;
            for s in body {
                ch |= replace_ident_slice_call_stmt(s, target, expected_obj_keys);
            }
            for s in handler {
                ch |= replace_ident_slice_call_stmt(s, target, expected_obj_keys);
            }
            ch
        }
        _ => false,
    }
}

fn elide_fresh_slice_in_block(
    stmts: &mut [Stmt],
    local_names: &HashSet<String>,
    reads: &HashMap<String, usize>,
    writes: &HashMap<String, usize>,
) -> bool {
    let mut changed = false;
    let mut fresh_vars: HashSet<String> = HashSet::new();
    let mut fresh_obj_keys: HashMap<String, Vec<String>> = HashMap::new();
    for i in 0..stmts.len() {
        // Recurse into child blocks first.
        match &mut stmts[i] {
            Stmt::If(_, a, b) => {
                changed |= elide_fresh_slice_in_block(a, local_names, reads, writes);
                if let Some(b) = b {
                    changed |= elide_fresh_slice_in_block(b, local_names, reads, writes);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    changed |= elide_fresh_slice_in_block(&mut c.body, local_names, reads, writes);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                changed |= elide_fresh_slice_in_block(b, local_names, reads, writes);
            }
            Stmt::Labeled(_, inner) => {
                changed |= elide_fresh_slice_in_block(
                    std::slice::from_mut(inner.as_mut()),
                    local_names,
                    reads,
                    writes,
                );
            }
            Stmt::TryCatch { body, handler, .. } => {
                changed |= elide_fresh_slice_in_block(body, local_names, reads, writes);
                changed |= elide_fresh_slice_in_block(handler, local_names, reads, writes);
            }
            _ => {}
        }
        if let Some((v, obj_keys)) =
            extract_fresh_array_def(&stmts[i], &fresh_vars, &fresh_obj_keys)
        {
            if local_names.contains(&v)
                && writes.get(&v).copied() == Some(1)
                && reads.get(&v).copied() == Some(1)
            {
                fresh_vars.insert(v.clone());
                if let Some(ref k) = obj_keys {
                    fresh_obj_keys.insert(v.clone(), k.clone());
                }
                // Replace the single `v.slice()` or pure shallow-object-copy `v.map(...)` in
                // trailing sibling statements.
                for tail in &mut stmts[i + 1..] {
                    if replace_ident_slice_call_stmt(tail, &v, obj_keys.as_deref()) {
                        changed = true;
                        break;
                    }
                }
            }
        }
    }
    changed
}

// =================================================================================================
// 3. Small Constant-Trip Loop Unrolling (`2..=10` iters, bottom-up)
// =================================================================================================

pub(super) fn unroll_small_const_loops(stmts: &mut Vec<Stmt>) -> bool {
    let mut changed = false;
    // Bottom-up: unroll inner loops first so outer small loops (like `sha3::permute` `m=0..5`
    // containing `n=0..5`) can also unroll cleanly.
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::If(_, a, b) => {
                changed |= unroll_small_const_loops(a);
                if let Some(b) = b {
                    changed |= unroll_small_const_loops(b);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    changed |= unroll_small_const_loops(&mut c.body);
                }
            }
            Stmt::LoopForever(_, b) | Stmt::While(_, b) | Stmt::Block(b) => {
                changed |= unroll_small_const_loops(b);
            }
            Stmt::Labeled(_, inner) => match inner.as_mut() {
                Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    changed |= unroll_small_const_loops(b);
                }
                Stmt::If(_, a, b) => {
                    changed |= unroll_small_const_loops(a);
                    if let Some(b) = b {
                        changed |= unroll_small_const_loops(b);
                    }
                }
                Stmt::Switch(_, cases) => {
                    for c in cases {
                        changed |= unroll_small_const_loops(&mut c.body);
                    }
                }
                _ => {}
            },
            Stmt::TryCatch { body, handler, .. } => {
                changed |= unroll_small_const_loops(body);
                changed |= unroll_small_const_loops(handler);
            }
            _ => {}
        }
    }

    let mut i = 0;
    while i < stmts.len() {
        let Some(unrolled) = try_unroll_loop_at(stmts, i) else {
            i += 1;
            continue;
        };
        let new_len = unrolled.len();
        stmts.splice(i..=i, unrolled);
        i += new_len;
        changed = true;
    }
    changed
}

struct ParsedConstLoop {
    iv: String,
    flag: Option<String>,
    c_start: u32,
    c_end: u32,
    /// Statements inside the loop body (with the final `iv = next` and optional `next = iv + 1`
    /// already stripped).
    iter_body: Vec<Stmt>,
    /// Exit statements from the `else { ...; break; }` branch.
    exit_stmts: Vec<Stmt>,
}

fn find_prior_const_init(stmts: &[Stmt], loop_idx: usize, iv: &str) -> Option<u32> {
    let mut k = loop_idx;
    let mut steps = 0;
    while k > 0 && steps < 4 {
        k -= 1;
        steps += 1;
        match &stmts[k] {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if matches!(lhs.as_ref(), Expr::Ident(id) if id == iv) {
                    return exact_u32(rhs);
                }
                if expr_has_non_panic_call(rhs) || expr_reads_ident(rhs, iv) {
                    return None;
                }
                if !matches!(lhs.as_ref(), Expr::Ident(_)) {
                    return None;
                }
            }
            _ => return None,
        }
    }
    None
}

fn is_iv_plus_one(expr: &Expr, iv: &str) -> bool {
    let inner = match expr {
        Expr::Binary(BinOp::UShr, l, r) if exact_u32(r) == Some(0) => l.as_ref(),
        Expr::Binary(BinOp::BitOr, l, r) if exact_i32(r) == Some(0) => l.as_ref(),
        other => other,
    };
    matches!(
        inner,
        Expr::Binary(BinOp::Add, l, r)
            if (matches!(l.as_ref(), Expr::Ident(id) if id == iv) && exact_u32(r) == Some(1))
                || (exact_u32(l) == Some(1) && matches!(r.as_ref(), Expr::Ident(id) if id == iv))
    )
}

fn try_unroll_loop_at(stmts: &[Stmt], loop_idx: usize) -> Option<Vec<Stmt>> {
    let parsed = parse_small_const_loop(stmts, loop_idx)?;
    let trip = parsed.c_end - parsed.c_start;
    if !(2..=10).contains(&trip) {
        return None;
    }
    let body_cost = count_stmts(&parsed.iter_body);
    if body_cost == 0 || (trip as usize) * body_cost > 160 {
        return None;
    }

    let mut out = Vec::with_capacity((trip as usize) * parsed.iter_body.len() + 3);
    for k in parsed.c_start..parsed.c_end {
        for s in &parsed.iter_body {
            let mut cloned = s.clone();
            subst_ident_in_stmt(&mut cloned, &parsed.iv, &Expr::Num(f64::from(k)));
            out.push(cloned);
        }
    }
    out.push(jsast::assign_stmt(
        jsast::id(&parsed.iv),
        Expr::Num(f64::from(parsed.c_end)),
    ));
    if let Some(flag) = parsed.flag {
        out.push(jsast::assign_stmt(jsast::id(flag), Expr::Bool(false)));
    }
    out.extend(parsed.exit_stmts);
    Some(out)
}

fn parse_small_const_loop(stmts: &[Stmt], loop_idx: usize) -> Option<ParsedConstLoop> {
    let (iv, flag, c_end, then_body, exit_stmts) = match &stmts[loop_idx] {
        Stmt::While(Expr::Binary(BinOp::Lt, l, r), body) => {
            let Expr::Ident(iv) = l.as_ref() else {
                return None;
            };
            let c_end = exact_u32(r)?;
            (iv.clone(), None, c_end, body.clone(), Vec::new())
        }
        Stmt::LoopForever(None, loop_body) => {
            // Form A: `[if (iv < C_end) { ... } else { exit...; break; }]`
            // Form B: `[flag = iv < C_end; if (flag) { ... } else { exit...; break; }]`
            let (iv, flag, c_end, then_b, else_b) = match loop_body.as_slice() {
                [Stmt::If(Expr::Binary(BinOp::Lt, l, r), then_b, Some(else_b))] => {
                    let Expr::Ident(iv) = l.as_ref() else {
                        return None;
                    };
                    (iv.clone(), None, exact_u32(r)?, then_b, else_b)
                }
                [Stmt::ExprStmt(Expr::Assign(flag_lhs, flag_rhs)), Stmt::If(Expr::Ident(flag_use), then_b, Some(else_b))] =>
                {
                    let Expr::Ident(flag_name) = flag_lhs.as_ref() else {
                        return None;
                    };
                    if flag_name != flag_use {
                        return None;
                    }
                    let Expr::Binary(BinOp::Lt, l, r) = flag_rhs.as_ref() else {
                        return None;
                    };
                    let Expr::Ident(iv) = l.as_ref() else {
                        return None;
                    };
                    (
                        iv.clone(),
                        Some(flag_name.clone()),
                        exact_u32(r)?,
                        then_b,
                        else_b,
                    )
                }
                _ => return None,
            };
            if !matches!(else_b.last(), Some(Stmt::Break(None))) {
                return None;
            }
            let exit = else_b[..else_b.len() - 1].to_vec();
            if stmts_have_control_flow(&exit) || stmts_have_non_panic_call(&exit) {
                return None;
            }
            (iv, flag, c_end, then_b.clone(), exit)
        }
        _ => return None,
    };

    let c_start = find_prior_const_init(stmts, loop_idx, &iv)?;
    if c_start >= c_end {
        return None;
    }
    if stmts_have_control_flow(&then_body) || stmts_have_non_panic_call(&then_body) {
        return None;
    }

    // Verify the final statement increments `iv` by 1 and `iv` is not written anywhere else.
    let mut iter_body = then_body;
    let last = iter_body.pop()?;
    let Stmt::ExprStmt(Expr::Assign(last_lhs, last_rhs)) = last else {
        return None;
    };
    if !matches!(last_lhs.as_ref(), Expr::Ident(id) if id == &iv) {
        return None;
    }
    if is_iv_plus_one(&last_rhs, &iv) {
        // Direct `iv = iv + 1 >>> 0;`
    } else if let Expr::Ident(next_var) = last_rhs.as_ref() {
        // `next = iv + 1 >>> 0;` as first statement of `iter_body`, never modified in `iter_body`.
        let first_matches = matches!(
            iter_body.first(),
            Some(Stmt::ExprStmt(Expr::Assign(fl, fr)))
                if matches!(fl.as_ref(), Expr::Ident(id) if id == next_var)
                    && is_iv_plus_one(fr, &iv)
        );
        if !first_matches {
            return None;
        }
        let mut rest_written = HashSet::new();
        collect_written_idents_stmts(&iter_body[1..], &mut rest_written, false);
        if rest_written.contains(next_var) {
            return None;
        }
        // Only remove `next = iv + 1` if `next` is not read anywhere else in `iter_body[1..]` or
        // `exit_stmts` (if it is read, keep it so `next` gets substituted with `k + 1`!).
        let next_read_in_body = stmts_read_ident(&iter_body[1..], next_var);
        let next_read_after = stmts_read_ident(&exit_stmts, next_var)
            || stmts_read_ident(&stmts[loop_idx + 1..], next_var);
        if !next_read_in_body && !next_read_after {
            iter_body.remove(0);
        }
    } else {
        return None;
    }

    // Ensure `iv` is never modified inside `iter_body`, and no `let`/`const` declarations exist
    // inside `iter_body` that would collide across unrolled copies.
    let mut body_written = HashSet::new();
    collect_written_idents_stmts(&iter_body, &mut body_written, false);
    if body_written.contains(&iv) || stmts_have_var_decl(&iter_body) {
        return None;
    }

    Some(ParsedConstLoop {
        iv,
        flag,
        c_start,
        c_end,
        iter_body,
        exit_stmts,
    })
}

// =================================================================================================
// 4. Dominated Redundant Guard Elimination & Constant-Table / Bitmask Bounds
// =================================================================================================

#[derive(Clone, Default)]
struct GuardEnv {
    known_true: HashSet<String>,
    known_false: HashSet<String>,
    /// Pairs `(idx_key, len_key)` proven `idx < len`.
    known_lt: HashSet<(String, String)>,
    /// Maps bool var `H` -> `(idx_key, len_key)` when `H = idx < len`.
    bool_lt_def: HashMap<String, (String, String)>,
    /// Maps local var `v` -> proven non-negative integer upper bound `<= u32`.
    upper_bounds: HashMap<String, u32>,
}

impl GuardEnv {
    fn invalidate(&mut self, ident: &str) {
        self.known_true.remove(ident);
        self.known_false.remove(ident);
        self.upper_bounds.remove(ident);
        self.bool_lt_def.remove(ident);
        self.bool_lt_def
            .retain(|_, (a, b)| a != ident && b != ident);
        self.known_lt.retain(|(a, b)| a != ident && b != ident);
    }

    fn record_cond_true(&mut self, cond: &Expr) {
        match cond {
            Expr::Ident(h) => {
                self.known_true.insert(h.clone());
                if let Some((a, b)) = self.bool_lt_def.get(h).cloned() {
                    self.known_lt.insert((a.clone(), b.clone()));
                    if let Ok(c) = b.parse::<u32>() {
                        if c >= 1 {
                            self.tighten_upper(&a, c - 1);
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
                if let (Some(lk), Some(rk)) = (simple_cmp_key(l), simple_cmp_key(r)) {
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
                Expr::Ident(h) => self.known_true.contains(h),
                Expr::Binary(BinOp::Lt, l, r) => {
                    if let (Some(lk), Some(rk)) = (simple_cmp_key(l), simple_cmp_key(r)) {
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

fn simple_cmp_key(expr: &Expr) -> Option<String> {
    if let Some(c) = exact_u32(expr) {
        return Some(c.to_string());
    }
    match expr {
        Expr::Ident(id) => Some(id.clone()),
        _ => None,
    }
}

pub(super) fn eliminate_dominated_guards(stmts: &mut Vec<Stmt>) {
    if stmts_have_mutating_nested_fn(stmts) {
        return;
    }
    let mut env = GuardEnv::default();
    elim_guards_in_block(stmts, &mut env);
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

        match &mut stmts[i] {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if let Expr::Ident(dst) = lhs.as_ref() {
                    let dst = dst.clone();
                    let ub = env.expr_upper_bound(rhs);
                    let lt_pair = match rhs.as_ref() {
                        Expr::Binary(BinOp::Lt, l, r) => simple_cmp_key(l).zip(simple_cmp_key(r)),
                        _ => None,
                    };
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
                        if a != dst && b != dst {
                            env.bool_lt_def.insert(dst, (a, b));
                        }
                    }
                } else {
                    let mut written = HashSet::new();
                    collect_written_idents_expr(lhs, &mut written);
                    collect_written_idents_expr(rhs, &mut written);
                    for w in written {
                        env.invalidate(&w);
                    }
                }
            }
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                let mut written = HashSet::new();
                collect_written_idents_expr(e, &mut written);
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::Let(n, init) => {
                let n = n.clone();
                if let Some(e) = init {
                    let mut written = HashSet::new();
                    collect_written_idents_expr(e, &mut written);
                    for w in written {
                        env.invalidate(&w);
                    }
                }
                env.invalidate(&n);
            }
            Stmt::Const(n, e) => {
                let n = n.clone();
                let mut written = HashSet::new();
                collect_written_idents_expr(e, &mut written);
                for w in written {
                    env.invalidate(&w);
                }
                env.invalidate(&n);
            }
            Stmt::Lets(decls) => {
                let mut written = HashSet::new();
                for (n, init) in decls {
                    written.insert(n.clone());
                    if let Some(e) = init {
                        collect_written_idents_expr(e, &mut written);
                    }
                }
                for w in written {
                    env.invalidate(&w);
                }
            }
            Stmt::If(cond, then_b, else_b) => {
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
                    let mut else_env = env.clone();
                    if let Expr::Unary(UnOp::Not, inner) = cond {
                        else_env.record_cond_true(inner);
                    } else if let Expr::Ident(b) = cond {
                        else_env.known_false.insert(b.clone());
                    }
                    elim_guards_in_block(eb, &mut else_env);
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(then_b, &mut written, true);
                    collect_written_idents_stmts(eb, &mut written, true);
                    for w in written {
                        env.invalidate(&w);
                    }
                } else if then_diverges {
                    // Cold diverging guard `if (cond) { panic; }` fell through: `!cond` holds!
                    if let Expr::Unary(UnOp::Not, inner) = cond {
                        env.record_cond_true(inner);
                    } else if let Expr::Ident(b) = cond {
                        env.known_false.insert(b.clone());
                    }
                } else {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(then_b, &mut written, true);
                    for w in written {
                        env.invalidate(&w);
                    }
                }
            }
            Stmt::Switch(disc, cases) => {
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
            Stmt::LoopForever(_, b) => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(b, &mut written, true);
                for w in written {
                    env.invalidate(&w);
                }
                let mut b_env = env.clone();
                elim_guards_in_block(b, &mut b_env);
            }
            Stmt::While(cond, b) => {
                let mut written = HashSet::new();
                collect_written_idents_expr(cond, &mut written);
                collect_written_idents_stmts(b, &mut written, true);
                for w in written {
                    env.invalidate(&w);
                }
                let mut b_env = env.clone();
                b_env.record_cond_true(cond);
                elim_guards_in_block(b, &mut b_env);
            }
            Stmt::Block(b) => {
                elim_guards_in_block(b, env);
            }
            Stmt::Labeled(_, inner) => match inner.as_mut() {
                Stmt::Block(b) => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(b, &mut written, false);
                    let mut b_env = env.clone();
                    elim_guards_in_block(b, &mut b_env);
                    for w in written {
                        env.invalidate(&w);
                    }
                }
                Stmt::LoopForever(_, b) => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(b, &mut written, true);
                    for w in written {
                        env.invalidate(&w);
                    }
                    let mut b_env = env.clone();
                    elim_guards_in_block(b, &mut b_env);
                }
                Stmt::While(cond, b) => {
                    let mut written = HashSet::new();
                    collect_written_idents_expr(cond, &mut written);
                    collect_written_idents_stmts(b, &mut written, true);
                    for w in written {
                        env.invalidate(&w);
                    }
                    let mut b_env = env.clone();
                    b_env.record_cond_true(cond);
                    elim_guards_in_block(b, &mut b_env);
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
// 5. Loop-Invariant Property Hoisting (LICM across cold diverging calls)
// =================================================================================================

pub(super) fn hoist_loop_invariant_props(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
) {
    if stmts_have_mutating_nested_fn(stmts) {
        return;
    }
    let mut escaped_locals: HashSet<String> = HashSet::new();
    collect_escaped_idents_stmts(stmts, &mut escaped_locals);
    let mut non_null: HashSet<String> = HashSet::new();
    hoist_props_in_block(
        stmts,
        local_names,
        extra_decls,
        &escaped_locals,
        &mut non_null,
    );
}

fn is_slice_prop(prop: &str) -> bool {
    matches!(prop, "buf" | "off" | "len")
}

fn is_guaranteed_non_null_rhs(expr: &Expr) -> bool {
    match expr {
        Expr::Object(_) | Expr::Array(_) | Expr::New(..) => true,
        Expr::Call(callee, _) => match callee.as_ref() {
            Expr::Ident(n) => {
                n.contains("_deref$")
                    || n.contains("_deref_mut$")
                    || n.contains("_as_slice$")
                    || n.contains("_as_mut_slice$")
                    || n.contains("_index$")
                    || n.contains("_index_mut$")
                    || n.contains("$deref$")
                    || n.contains("$deref_mut$")
                    || n.contains("$as_slice$")
                    || n.contains("$as_mut_slice$")
                    || n == "$rt_sfp"
                    || n == "$rt_bstr_bytes"
                    || n == "$rt_str_bytes"
            }
            Expr::Member(obj, prop) => {
                matches!(obj.as_ref(), Expr::Ident(m) if m == "__rt")
                    && matches!(prop.as_str(), "sfp" | "bstr_bytes" | "str_bytes")
            }
            _ => false,
        },
        _ => false,
    }
}

fn collect_unconditional_prop_bases_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Member(obj, _) => {
            if let Expr::Ident(id) = obj.as_ref() {
                out.insert(id.clone());
            }
            collect_unconditional_prop_bases_expr(obj, out);
        }
        Expr::Index(o, i) => {
            collect_unconditional_prop_bases_expr(o, out);
            collect_unconditional_prop_bases_expr(i, out);
        }
        Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            collect_unconditional_prop_bases_expr(r, out);
            if !matches!(l.as_ref(), Expr::Ident(_)) {
                collect_unconditional_prop_bases_expr(l, out);
            }
        }
        Expr::Binary(op, l, r) => {
            collect_unconditional_prop_bases_expr(l, out);
            if !matches!(op, BinOp::And | BinOp::Or) {
                collect_unconditional_prop_bases_expr(r, out);
            }
        }
        Expr::Unary(_, e) => collect_unconditional_prop_bases_expr(e, out),
        Expr::Cond(c, _, _) => collect_unconditional_prop_bases_expr(c, out),
        Expr::Call(c, args) | Expr::New(c, args) => {
            collect_unconditional_prop_bases_expr(c, out);
            for a in args {
                collect_unconditional_prop_bases_expr(a, out);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                collect_unconditional_prop_bases_expr(x, out);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_unconditional_prop_bases_expr(v, out);
            }
        }
        _ => {}
    }
}

fn hoist_props_in_block(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
    escaped_locals: &HashSet<String>,
    non_null: &mut HashSet<String>,
) {
    let mut i = 0;
    while i < stmts.len() {
        // First recurse into nested structures (innermost loops first!).
        match &mut stmts[i] {
            Stmt::If(cond, a, b) => {
                collect_unconditional_prop_bases_expr(cond, non_null);
                let mut a_nn = non_null.clone();
                hoist_props_in_block(a, local_names, extra_decls, escaped_locals, &mut a_nn);
                if let Some(b) = b {
                    let mut b_nn = non_null.clone();
                    hoist_props_in_block(b, local_names, extra_decls, escaped_locals, &mut b_nn);
                }
                let mut written = HashSet::new();
                collect_written_idents_stmts(a, &mut written, true);
                if let Some(b) = b {
                    collect_written_idents_stmts(b, &mut written, true);
                }
                for w in written {
                    non_null.remove(&w);
                }
                i += 1;
                continue;
            }
            Stmt::Switch(disc, cases) => {
                collect_unconditional_prop_bases_expr(disc, non_null);
                let mut written = HashSet::new();
                for c in cases.iter_mut() {
                    let mut c_nn = non_null.clone();
                    hoist_props_in_block(
                        &mut c.body,
                        local_names,
                        extra_decls,
                        escaped_locals,
                        &mut c_nn,
                    );
                    collect_written_idents_stmts(&c.body, &mut written, true);
                }
                for w in written {
                    non_null.remove(&w);
                }
                i += 1;
                continue;
            }
            Stmt::Block(b) => {
                hoist_props_in_block(b, local_names, extra_decls, escaped_locals, non_null);
                i += 1;
                continue;
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut b_nn = non_null.clone();
                hoist_props_in_block(body, local_names, extra_decls, escaped_locals, &mut b_nn);
                let mut written = HashSet::new();
                collect_written_idents_stmts(body, &mut written, false);
                for w in written {
                    non_null.remove(&w);
                }
                let mut h_nn = non_null.clone();
                hoist_props_in_block(handler, local_names, extra_decls, escaped_locals, &mut h_nn);
                let mut h_written = HashSet::new();
                collect_written_idents_stmts(handler, &mut h_written, false);
                for w in h_written {
                    non_null.remove(&w);
                }
                i += 1;
                continue;
            }
            Stmt::LoopForever(_, b) => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(b, &mut written, true);
                let mut inner_nn = non_null.clone();
                for w in &written {
                    inner_nn.remove(w);
                }
                hoist_props_in_block(b, local_names, extra_decls, escaped_locals, &mut inner_nn);
            }
            Stmt::While(cond, b) => {
                let mut written = HashSet::new();
                collect_written_idents_expr(cond, &mut written);
                collect_written_idents_stmts(b, &mut written, true);
                let mut inner_nn = non_null.clone();
                for w in &written {
                    inner_nn.remove(w);
                }
                hoist_props_in_block(b, local_names, extra_decls, escaped_locals, &mut inner_nn);
            }
            Stmt::Labeled(_, inner) => match inner.as_mut() {
                Stmt::Block(b) => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(b, &mut written, false);
                    let mut b_nn = non_null.clone();
                    hoist_props_in_block(b, local_names, extra_decls, escaped_locals, &mut b_nn);
                    for w in written {
                        non_null.remove(&w);
                    }
                    i += 1;
                    continue;
                }
                Stmt::LoopForever(_, b) => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(b, &mut written, true);
                    let mut inner_nn = non_null.clone();
                    for w in &written {
                        inner_nn.remove(w);
                    }
                    hoist_props_in_block(
                        b,
                        local_names,
                        extra_decls,
                        escaped_locals,
                        &mut inner_nn,
                    );
                }
                Stmt::While(cond, b) => {
                    let mut written = HashSet::new();
                    collect_written_idents_expr(cond, &mut written);
                    collect_written_idents_stmts(b, &mut written, true);
                    let mut inner_nn = non_null.clone();
                    for w in &written {
                        inner_nn.remove(w);
                    }
                    hoist_props_in_block(
                        b,
                        local_names,
                        extra_decls,
                        escaped_locals,
                        &mut inner_nn,
                    );
                }
                _ => {
                    let mut written = HashSet::new();
                    collect_written_idents_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        &mut written,
                        true,
                    );
                    for w in written {
                        non_null.remove(&w);
                    }
                    i += 1;
                    continue;
                }
            },
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                collect_unconditional_prop_bases_expr(rhs, non_null);
                if let Expr::Ident(dst) = lhs.as_ref() {
                    if is_guaranteed_non_null_rhs(rhs) {
                        non_null.insert(dst.clone());
                    } else {
                        non_null.remove(dst);
                    }
                } else {
                    collect_unconditional_prop_bases_expr(lhs, non_null);
                }
                i += 1;
                continue;
            }
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                collect_unconditional_prop_bases_expr(e, non_null);
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        }

        // Now `stmts[i]` is a `LoopForever` or `While` (optionally wrapped in `Stmt::Labeled`).
        // Check for hoistable `.buf`/`.off`/`.len`.
        let mut assigned_in_loop = HashSet::new();
        let mut props_written_in_loop = HashSet::new();
        let mut prop_reads: HashMap<(String, String), usize> = HashMap::new();
        let loop_inner = match &stmts[i] {
            Stmt::Labeled(_, inner) => inner.as_ref(),
            other => other,
        };
        let has_non_panic = match loop_inner {
            Stmt::LoopForever(_, b) => {
                collect_written_idents_stmts(b, &mut assigned_in_loop, true);
                collect_prop_writes_stmts(b, &mut props_written_in_loop);
                collect_slice_prop_reads_stmts(b, &mut prop_reads);
                stmts_have_non_panic_call(b)
            }
            Stmt::While(c, b) => {
                collect_written_idents_expr(c, &mut assigned_in_loop);
                collect_written_idents_stmts(b, &mut assigned_in_loop, true);
                collect_prop_writes_expr(c, &mut props_written_in_loop);
                collect_prop_writes_stmts(b, &mut props_written_in_loop);
                collect_slice_prop_reads_expr(c, &mut prop_reads);
                collect_slice_prop_reads_stmts(b, &mut prop_reads);
                expr_has_non_panic_call(c) || stmts_have_non_panic_call(b)
            }
            _ => unreachable!(),
        };
        let has_panic = stmt_has_panic_call(&stmts[i]);
        // If a constant-trip loop (`C_start < C_end`) is guaranteed to execute iteration 0, any
        // object whose property is read unconditionally before the first side effect/branch in
        // iteration 0 is guaranteed non-nullish on entry (and stays non-nullish after).
        let mut loop_non_null = non_null.clone();
        if let Some(c) = detect_loop_counter(stmts, i, &assigned_in_loop) {
            if let (Some(c_start), Some(c_end)) = (c.iv_min, exact_u32(&c.bound)) {
                if c_start < c_end {
                    let iter0_body: Option<&[Stmt]> = match loop_inner {
                        Stmt::While(_, b) => Some(b.as_slice()),
                        Stmt::LoopForever(_, b) => match b.as_slice() {
                            [Stmt::If(_, then_b, _)] | [_, Stmt::If(_, then_b, _)] => {
                                Some(then_b.as_slice())
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(body0) = iter0_body {
                        for s in body0 {
                            match s {
                                Stmt::ExprStmt(e) if !expr_has_non_panic_call(e) => {
                                    collect_unconditional_prop_bases_expr(e, &mut loop_non_null);
                                }
                                Stmt::If(cond, then_b, None)
                                    if stmts_definitely_diverge(then_b)
                                        && !expr_has_non_panic_call(cond) =>
                                {
                                    collect_unconditional_prop_bases_expr(cond, &mut loop_non_null);
                                }
                                _ => break,
                            }
                        }
                    }
                }
            }
        }

        let mut candidates: Vec<(String, String)> = prop_reads
            .into_iter()
            .filter_map(|((obj, prop), count)| {
                if !is_slice_prop(&prop)
                    || assigned_in_loop.contains(&obj)
                    || props_written_in_loop.contains(&(obj.clone(), prop.clone()))
                    || !loop_non_null.contains(&obj)
                {
                    return None;
                }
                if has_non_panic && (!local_names.contains(&obj) || escaped_locals.contains(&obj)) {
                    return None;
                }
                if count >= 2 || has_panic {
                    Some((obj, prop))
                } else {
                    None
                }
            })
            .collect();
        candidates.sort();

        if !candidates.is_empty() {
            let mut hoisted_stmts = Vec::with_capacity(candidates.len());
            let mut subst: HashMap<(String, String), String> = HashMap::new();
            for (obj, prop) in candidates {
                let prefix = match prop.as_str() {
                    "buf" => "$hb_",
                    "off" => "$ho_",
                    _ => "$hl_",
                };
                let tmp = alloc_fresh_local(local_names, extra_decls, prefix);
                hoisted_stmts.push(jsast::assign_stmt(
                    jsast::id(&tmp),
                    Expr::Member(Box::new(Expr::Ident(obj.clone())), prop.clone()),
                ));
                subst.insert((obj, prop), tmp);
            }
            replace_hoisted_props_stmt(&mut stmts[i], &subst);
            let n = hoisted_stmts.len();
            stmts.splice(i..i, hoisted_stmts);
            i += n;
        }

        for w in assigned_in_loop {
            non_null.remove(&w);
        }
        i += 1;
    }
}

fn collect_escaped_idents_expr(expr: &Expr, escaped: &mut HashSet<String>) {
    match expr {
        Expr::Member(obj, prop)
            if is_slice_prop(prop) && matches!(obj.as_ref(), Expr::Ident(_)) =>
        {
            // Pure `.buf`/`.off`/`.len` read does NOT escape `obj`.
        }
        Expr::Ident(id) => {
            escaped.insert(id.clone());
        }
        Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
            // Assigning `x = rhs` does not escape `x`; only `rhs` may escape if it's an ident.
            if !matches!(l.as_ref(), Expr::Ident(_)) {
                collect_escaped_idents_expr(l, escaped);
            }
            if !matches!(r.as_ref(), Expr::Member(o, p) if is_slice_prop(p) && matches!(o.as_ref(), Expr::Ident(_)))
            {
                collect_escaped_idents_expr(r, escaped);
            }
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => collect_escaped_idents_expr(e, escaped),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            collect_escaped_idents_expr(l, escaped);
            collect_escaped_idents_expr(r, escaped);
        }
        Expr::Cond(c, t, e) => {
            collect_escaped_idents_expr(c, escaped);
            collect_escaped_idents_expr(t, escaped);
            collect_escaped_idents_expr(e, escaped);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            let is_panic = matches!(c.as_ref(), Expr::Ident(n) if n.starts_with("core$panicking$"));
            if !is_panic {
                collect_escaped_idents_expr(c, escaped);
                for a in args {
                    collect_escaped_idents_expr(a, escaped);
                }
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                collect_escaped_idents_expr(x, escaped);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_escaped_idents_expr(v, escaped);
            }
        }
        Expr::Arrow(params, ArrowBody::Expr(body)) => {
            let mut inner = HashSet::new();
            collect_escaped_idents_expr(body, &mut inner);
            for id in inner {
                if !params.iter().any(|p| p == &id) {
                    escaped.insert(id);
                }
            }
        }
        _ => {}
    }
}

fn collect_escaped_idents_stmts(stmts: &[Stmt], escaped: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
                collect_escaped_idents_expr(e, escaped);
            }
            Stmt::Let(_, Some(e)) => collect_escaped_idents_expr(e, escaped),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        collect_escaped_idents_expr(e, escaped);
                    }
                }
            }
            Stmt::If(c, a, b) => {
                collect_escaped_idents_expr(c, escaped);
                if !stmts_definitely_diverge(a) {
                    collect_escaped_idents_stmts(a, escaped);
                }
                if let Some(b) = b {
                    if !stmts_definitely_diverge(b) {
                        collect_escaped_idents_stmts(b, escaped);
                    }
                }
            }
            Stmt::Switch(d, cases) => {
                collect_escaped_idents_expr(d, escaped);
                for c in cases {
                    collect_escaped_idents_stmts(&c.body, escaped);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) => collect_escaped_idents_stmts(b, escaped),
            Stmt::While(c, b) => {
                collect_escaped_idents_expr(c, escaped);
                collect_escaped_idents_stmts(b, escaped);
            }
            Stmt::Labeled(_, inner) => {
                collect_escaped_idents_stmts(std::slice::from_ref(inner.as_ref()), escaped);
            }
            Stmt::TryCatch { body, handler, .. } => {
                collect_escaped_idents_stmts(body, escaped);
                collect_escaped_idents_stmts(handler, escaped);
            }
            _ => {}
        }
    }
}

fn collect_prop_writes_expr(expr: &Expr, out: &mut HashSet<(String, String)>) {
    match expr {
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            if let Expr::Member(obj, prop) = lhs.as_ref() {
                if let Expr::Ident(id) = obj.as_ref() {
                    out.insert((id.clone(), prop.clone()));
                }
            }
            collect_prop_writes_expr(lhs, out);
            collect_prop_writes_expr(rhs, out);
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => collect_prop_writes_expr(e, out),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            collect_prop_writes_expr(l, out);
            collect_prop_writes_expr(r, out);
        }
        Expr::Cond(c, t, e) => {
            collect_prop_writes_expr(c, out);
            collect_prop_writes_expr(t, out);
            collect_prop_writes_expr(e, out);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            collect_prop_writes_expr(c, out);
            for a in args {
                collect_prop_writes_expr(a, out);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                collect_prop_writes_expr(x, out);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_prop_writes_expr(v, out);
            }
        }
        _ => {}
    }
}

fn collect_prop_writes_stmts(stmts: &[Stmt], out: &mut HashSet<(String, String)>) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
                collect_prop_writes_expr(e, out);
            }
            Stmt::Let(_, Some(e)) => collect_prop_writes_expr(e, out),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        collect_prop_writes_expr(e, out);
                    }
                }
            }
            Stmt::If(c, a, b) => {
                collect_prop_writes_expr(c, out);
                collect_prop_writes_stmts(a, out);
                if let Some(b) = b {
                    collect_prop_writes_stmts(b, out);
                }
            }
            Stmt::Switch(d, cases) => {
                collect_prop_writes_expr(d, out);
                for c in cases {
                    collect_prop_writes_stmts(&c.body, out);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) => collect_prop_writes_stmts(b, out),
            Stmt::While(c, b) => {
                collect_prop_writes_expr(c, out);
                collect_prop_writes_stmts(b, out);
            }
            Stmt::Labeled(_, inner) => {
                collect_prop_writes_stmts(std::slice::from_ref(inner.as_ref()), out);
            }
            Stmt::TryCatch { body, handler, .. } => {
                collect_prop_writes_stmts(body, out);
                collect_prop_writes_stmts(handler, out);
            }
            _ => {}
        }
    }
}

fn collect_slice_prop_reads_expr(expr: &Expr, out: &mut HashMap<(String, String), usize>) {
    match expr {
        Expr::Member(obj, prop) if is_slice_prop(prop) => {
            if let Expr::Ident(id) = obj.as_ref() {
                *out.entry((id.clone(), prop.clone())).or_default() += 1;
            }
            collect_slice_prop_reads_expr(obj, out);
        }
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            if !matches!(lhs.as_ref(), Expr::Member(_, _)) {
                collect_slice_prop_reads_expr(lhs, out);
            }
            collect_slice_prop_reads_expr(rhs, out);
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => collect_slice_prop_reads_expr(e, out),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            collect_slice_prop_reads_expr(l, out);
            collect_slice_prop_reads_expr(r, out);
        }
        Expr::Cond(c, t, e) => {
            collect_slice_prop_reads_expr(c, out);
            collect_slice_prop_reads_expr(t, out);
            collect_slice_prop_reads_expr(e, out);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            collect_slice_prop_reads_expr(c, out);
            for a in args {
                collect_slice_prop_reads_expr(a, out);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                collect_slice_prop_reads_expr(x, out);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_slice_prop_reads_expr(v, out);
            }
        }
        _ => {}
    }
}

fn collect_slice_prop_reads_stmts(stmts: &[Stmt], out: &mut HashMap<(String, String), usize>) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
                collect_slice_prop_reads_expr(e, out);
            }
            Stmt::Let(_, Some(e)) => collect_slice_prop_reads_expr(e, out),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        collect_slice_prop_reads_expr(e, out);
                    }
                }
            }
            Stmt::If(c, a, b) => {
                collect_slice_prop_reads_expr(c, out);
                if !stmts_definitely_diverge(a) {
                    collect_slice_prop_reads_stmts(a, out);
                }
                if let Some(b) = b {
                    if !stmts_definitely_diverge(b) {
                        collect_slice_prop_reads_stmts(b, out);
                    }
                }
            }
            Stmt::Switch(d, cases) => {
                collect_slice_prop_reads_expr(d, out);
                for c in cases {
                    collect_slice_prop_reads_stmts(&c.body, out);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) => collect_slice_prop_reads_stmts(b, out),
            Stmt::While(c, b) => {
                collect_slice_prop_reads_expr(c, out);
                collect_slice_prop_reads_stmts(b, out);
            }
            Stmt::Labeled(_, inner) => {
                collect_slice_prop_reads_stmts(std::slice::from_ref(inner.as_ref()), out);
            }
            Stmt::TryCatch { body, handler, .. } => {
                collect_slice_prop_reads_stmts(body, out);
                collect_slice_prop_reads_stmts(handler, out);
            }
            _ => {}
        }
    }
}

fn replace_hoisted_props_expr(expr: &mut Expr, subst: &HashMap<(String, String), String>) {
    if let Expr::Member(obj, prop) = expr {
        if let Expr::Ident(id) = obj.as_ref() {
            if let Some(tmp) = subst.get(&(id.clone(), prop.clone())) {
                *expr = Expr::Ident(tmp.clone());
                return;
            }
        }
    }
    match expr {
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            if !matches!(lhs.as_ref(), Expr::Member(_, _)) {
                replace_hoisted_props_expr(lhs, subst);
            }
            replace_hoisted_props_expr(rhs, subst);
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => replace_hoisted_props_expr(e, subst),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            replace_hoisted_props_expr(l, subst);
            replace_hoisted_props_expr(r, subst);
        }
        Expr::Cond(c, t, e) => {
            replace_hoisted_props_expr(c, subst);
            replace_hoisted_props_expr(t, subst);
            replace_hoisted_props_expr(e, subst);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            replace_hoisted_props_expr(c, subst);
            for a in args {
                replace_hoisted_props_expr(a, subst);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                replace_hoisted_props_expr(x, subst);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                replace_hoisted_props_expr(v, subst);
            }
        }
        _ => {}
    }
}

fn replace_hoisted_props_stmt(stmt: &mut Stmt, subst: &HashMap<(String, String), String>) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
            replace_hoisted_props_expr(e, subst);
        }
        Stmt::Let(_, Some(e)) => replace_hoisted_props_expr(e, subst),
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    replace_hoisted_props_expr(e, subst);
                }
            }
        }
        Stmt::If(c, a, b) => {
            replace_hoisted_props_expr(c, subst);
            for s in a {
                replace_hoisted_props_stmt(s, subst);
            }
            if let Some(b) = b {
                for s in b {
                    replace_hoisted_props_stmt(s, subst);
                }
            }
        }
        Stmt::Switch(d, cases) => {
            replace_hoisted_props_expr(d, subst);
            for c in cases {
                for s in &mut c.body {
                    replace_hoisted_props_stmt(s, subst);
                }
            }
        }
        Stmt::Block(b) | Stmt::LoopForever(_, b) => {
            for s in b {
                replace_hoisted_props_stmt(s, subst);
            }
        }
        Stmt::While(c, b) => {
            replace_hoisted_props_expr(c, subst);
            for s in b {
                replace_hoisted_props_stmt(s, subst);
            }
        }
        Stmt::Labeled(_, inner) => replace_hoisted_props_stmt(inner, subst),
        Stmt::TryCatch { body, handler, .. } => {
            for s in body {
                replace_hoisted_props_stmt(s, subst);
            }
            for s in handler {
                replace_hoisted_props_stmt(s, subst);
            }
        }
        _ => {}
    }
}

// =================================================================================================
// 6. Loop-Versioned BCE (Hot/Cold Pre-Loop Guard)
// =================================================================================================

pub(super) fn version_checked_loops(stmts: &mut [Stmt]) {
    if stmts_have_mutating_nested_fn(stmts) {
        return;
    }
    let mut active_masked: HashMap<String, Expr> = HashMap::new();
    version_loops_in_block(stmts, &mut active_masked);
}

#[derive(Clone)]
struct CounterInfo {
    iv: String,
    bound: Expr, // Ident or exact_u32 Num
    iv_min: Option<u32>,
}

fn invalidate_masked(active_masked: &mut HashMap<String, Expr>, ident: &str) {
    active_masked.remove(ident);
    active_masked.retain(|_, mask_expr| !expr_reads_ident(mask_expr, ident));
}

fn version_loops_in_block(stmts: &mut [Stmt], active_masked: &mut HashMap<String, Expr>) {
    // Walk statements in order, maintaining `active_masked` (variables whose current value is
    // `(... & mask) >>> 0`) so inner loops wrapped in `a: { for (;;) { ... } }` inherit pre-loop
    // masked initializations from their enclosing block, while still versioning inner loops first.
    let no_assigned: HashSet<String> = HashSet::new();
    for i in 0..stmts.len() {
        let pre_loop_masked = active_masked.clone();
        match &mut stmts[i] {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                let mut rhs_w = HashSet::new();
                collect_written_idents_expr(rhs, &mut rhs_w);
                for w in rhs_w {
                    invalidate_masked(active_masked, &w);
                }
                if let Expr::Ident(id) = lhs.as_ref() {
                    let id = id.clone();
                    let mask = extract_bitand_mask(rhs, &no_assigned)
                        .filter(|m| !expr_reads_ident(m, &id));
                    invalidate_masked(active_masked, &id);
                    if let Some(m) = mask {
                        active_masked.insert(id, m);
                    }
                } else {
                    let mut lhs_w = HashSet::new();
                    collect_written_idents_expr(lhs, &mut lhs_w);
                    for w in lhs_w {
                        invalidate_masked(active_masked, &w);
                    }
                }
            }
            Stmt::If(cond, a, b) => {
                let mut w = HashSet::new();
                collect_written_idents_expr(cond, &mut w);
                for k in &w {
                    invalidate_masked(active_masked, k);
                }
                let mut a_m = active_masked.clone();
                version_loops_in_block(a, &mut a_m);
                collect_written_idents_stmts(a, &mut w, true);
                if let Some(b) = b {
                    let mut b_m = active_masked.clone();
                    version_loops_in_block(b, &mut b_m);
                    collect_written_idents_stmts(b, &mut w, true);
                }
                for k in w {
                    invalidate_masked(active_masked, &k);
                }
            }
            Stmt::Switch(disc, cases) => {
                let mut w = HashSet::new();
                collect_written_idents_expr(disc, &mut w);
                for k in &w {
                    invalidate_masked(active_masked, k);
                }
                for c in cases {
                    let mut c_m = active_masked.clone();
                    version_loops_in_block(&mut c.body, &mut c_m);
                    collect_written_idents_stmts(&c.body, &mut w, true);
                }
                for k in w {
                    invalidate_masked(active_masked, &k);
                }
            }
            Stmt::Block(b) => {
                version_loops_in_block(b, active_masked);
            }
            Stmt::Labeled(_, inner) => match inner.as_mut() {
                Stmt::Block(b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(b, &mut w, false);
                    let mut b_m = active_masked.clone();
                    version_loops_in_block(b, &mut b_m);
                    for k in w {
                        invalidate_masked(active_masked, &k);
                    }
                }
                Stmt::LoopForever(_, b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(b, &mut w, true);
                    for k in &w {
                        invalidate_masked(active_masked, k);
                    }
                    let mut inner_m = active_masked.clone();
                    version_loops_in_block(b, &mut inner_m);
                }
                Stmt::While(c, b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_expr(c, &mut w);
                    collect_written_idents_stmts(b, &mut w, true);
                    for k in &w {
                        invalidate_masked(active_masked, k);
                    }
                    let mut inner_m = active_masked.clone();
                    version_loops_in_block(b, &mut inner_m);
                }
                other => {
                    let mut m = active_masked.clone();
                    version_loops_in_block(std::slice::from_mut(other), &mut m);
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(std::slice::from_ref(other), &mut w, true);
                    for k in w {
                        invalidate_masked(active_masked, &k);
                    }
                }
            },
            Stmt::LoopForever(_, b) => {
                let mut w = HashSet::new();
                collect_written_idents_stmts(b, &mut w, true);
                for k in &w {
                    invalidate_masked(active_masked, k);
                }
                let mut inner_m = active_masked.clone();
                version_loops_in_block(b, &mut inner_m);
            }
            Stmt::While(c, b) => {
                let mut w = HashSet::new();
                collect_written_idents_expr(c, &mut w);
                collect_written_idents_stmts(b, &mut w, true);
                for k in &w {
                    invalidate_masked(active_masked, k);
                }
                let mut inner_m = active_masked.clone();
                version_loops_in_block(b, &mut inner_m);
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut b_m = active_masked.clone();
                version_loops_in_block(body, &mut b_m);
                let mut w = HashSet::new();
                collect_written_idents_stmts(body, &mut w, false);
                for k in &w {
                    invalidate_masked(active_masked, k);
                }
                let mut h_m = active_masked.clone();
                version_loops_in_block(handler, &mut h_m);
                collect_written_idents_stmts(handler, &mut w, false);
                for k in w {
                    invalidate_masked(active_masked, &k);
                }
            }
            other => {
                let mut w = HashSet::new();
                collect_written_idents_stmts(std::slice::from_ref(other), &mut w, true);
                for k in w {
                    invalidate_masked(active_masked, &k);
                }
            }
        }

        if let Some(versioned) = try_version_loop_at(stmts, i, &pre_loop_masked) {
            stmts[i] = versioned;
        }
    }
}

fn is_loop_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::While(..) | Stmt::LoopForever(..) => true,
        Stmt::Labeled(_, inner) => {
            matches!(inner.as_ref(), Stmt::While(..) | Stmt::LoopForever(..))
        }
        _ => false,
    }
}

fn extract_loop_body(stmt: &Stmt) -> Option<&[Stmt]> {
    match stmt {
        Stmt::While(_, b) | Stmt::LoopForever(_, b) => Some(b.as_slice()),
        Stmt::Labeled(_, inner) => extract_loop_body(inner),
        _ => None,
    }
}

fn extract_loop_body_mut(stmt: &mut Stmt) -> Option<&mut Vec<Stmt>> {
    match stmt {
        Stmt::While(_, b) | Stmt::LoopForever(_, b) => Some(b),
        Stmt::Labeled(_, inner) => extract_loop_body_mut(inner),
        _ => None,
    }
}

fn detect_loop_counter(
    enclosing: &[Stmt],
    loop_idx: usize,
    assigned_in_loop: &HashSet<String>,
) -> Option<CounterInfo> {
    let stmt = match &enclosing[loop_idx] {
        Stmt::Labeled(_, inner) => inner.as_ref(),
        other => other,
    };
    let (iv, bound, body): (String, Expr, &[Stmt]) = match stmt {
        Stmt::While(Expr::Binary(BinOp::Lt, l, r), b) => {
            let Expr::Ident(iv) = l.as_ref() else {
                return None;
            };
            (iv.clone(), r.as_ref().clone(), b.as_slice())
        }
        Stmt::LoopForever(_, b) => match b.as_slice() {
            [Stmt::If(Expr::Binary(BinOp::Lt, l, r), then_b, Some(else_b))]
                if matches!(else_b.last(), Some(Stmt::Break(_))) =>
            {
                let Expr::Ident(iv) = l.as_ref() else {
                    return None;
                };
                (iv.clone(), r.as_ref().clone(), then_b.as_slice())
            }
            [Stmt::ExprStmt(Expr::Assign(fl, fr)), Stmt::If(Expr::Ident(fu), then_b, Some(else_b))]
                if matches!(fl.as_ref(), Expr::Ident(fn_) if fn_ == fu)
                    && matches!(else_b.last(), Some(Stmt::Break(_))) =>
            {
                let Expr::Binary(BinOp::Lt, l, r) = fr.as_ref() else {
                    return None;
                };
                let Expr::Ident(iv) = l.as_ref() else {
                    return None;
                };
                (iv.clone(), r.as_ref().clone(), then_b.as_slice())
            }
            _ => return None,
        },
        _ => return None,
    };
    // `bound` must be an exact u32 constant or a loop-invariant identifier.
    let bound_invariant = match &bound {
        Expr::Ident(b_id) => b_id != &iv && !assigned_in_loop.contains(b_id),
        other => exact_u32(other).is_some(),
    };
    if !bound_invariant {
        return None;
    }
    // Verify EVERY assignment to `iv` inside `body` is a monotonic `+ 1` step.
    if !all_iv_writes_are_plus_one(body, &iv) {
        return None;
    }
    let iv_min = find_prior_const_init(enclosing, loop_idx, &iv);
    Some(CounterInfo { iv, bound, iv_min })
}

fn all_iv_writes_are_plus_one(stmts: &[Stmt], iv: &str) -> bool {
    let mut local_plus_one: HashSet<String> = HashSet::new();
    let mut saw_write = false;
    fn check_stmts(
        stmts: &[Stmt],
        iv: &str,
        local_plus_one: &mut HashSet<String>,
        saw_write: &mut bool,
    ) -> bool {
        for s in stmts {
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if expr_writes_ident(rhs, iv) {
                        return false;
                    }
                    if let Expr::Ident(id) = lhs.as_ref() {
                        if id == iv {
                            *saw_write = true;
                            let ok = is_iv_plus_one(rhs, iv)
                                || matches!(rhs.as_ref(), Expr::Ident(n) if local_plus_one.contains(n));
                            if !ok {
                                return false;
                            }
                            local_plus_one.clear();
                        } else if is_iv_plus_one(rhs, iv) {
                            local_plus_one.insert(id.clone());
                        } else {
                            local_plus_one.remove(id);
                        }
                    } else if expr_writes_ident(lhs, iv) {
                        return false;
                    }
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                    if expr_writes_ident(e, iv) {
                        return false;
                    }
                }
                Stmt::If(c, a, b) => {
                    if expr_writes_ident(c, iv) {
                        return false;
                    }
                    let mut a_p = local_plus_one.clone();
                    if !check_stmts(a, iv, &mut a_p, saw_write) {
                        return false;
                    }
                    if let Some(b) = b {
                        let mut b_p = local_plus_one.clone();
                        if !check_stmts(b, iv, &mut b_p, saw_write) {
                            return false;
                        }
                    }
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(a, &mut w, true);
                    if let Some(b) = b {
                        collect_written_idents_stmts(b, &mut w, true);
                    }
                    for k in w {
                        local_plus_one.remove(&k);
                    }
                }
                Stmt::Block(b) => {
                    if !check_stmts(b, iv, local_plus_one, saw_write) {
                        return false;
                    }
                }
                Stmt::Labeled(_, inner) => {
                    if !check_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        iv,
                        local_plus_one,
                        saw_write,
                    ) {
                        return false;
                    }
                }
                Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    let mut w = HashSet::new();
                    collect_written_idents_stmts(b, &mut w, true);
                    if w.contains(iv) {
                        return false;
                    }
                    for k in w {
                        local_plus_one.remove(&k);
                    }
                }
                Stmt::Switch(d, cases) => {
                    if expr_writes_ident(d, iv) {
                        return false;
                    }
                    for c in cases {
                        let mut w = HashSet::new();
                        collect_written_idents_stmts(&c.body, &mut w, true);
                        if w.contains(iv) {
                            return false;
                        }
                    }
                    local_plus_one.clear();
                }
                _ => {}
            }
        }
        true
    }
    check_stmts(stmts, iv, &mut local_plus_one, &mut saw_write) && saw_write
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
    let mut masks: HashMap<String, Option<Expr>> = HashMap::new();
    fn scan_writes(
        stmts: &[Stmt],
        assigned_in_loop: &HashSet<String>,
        masks: &mut HashMap<String, Option<Expr>>,
    ) {
        for s in stmts {
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if let Expr::Ident(id) = lhs.as_ref() {
                        let m = extract_bitand_mask(rhs, assigned_in_loop);
                        masks
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
                        scan_writes(a, assigned_in_loop, masks);
                    }
                    if let Some(b) = b {
                        if !stmts_definitely_diverge(b) {
                            scan_writes(b, assigned_in_loop, masks);
                        }
                    }
                }
                Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
                    scan_writes(b, assigned_in_loop, masks);
                }
                Stmt::Labeled(_, inner) => {
                    scan_writes(
                        std::slice::from_ref(inner.as_ref()),
                        assigned_in_loop,
                        masks,
                    );
                }
                Stmt::Switch(_, cases) => {
                    for c in cases {
                        scan_writes(&c.body, assigned_in_loop, masks);
                    }
                }
                _ => {}
            }
        }
    }
    scan_writes(loop_body, assigned_in_loop, &mut masks);

    let mut confirmed = HashMap::new();
    for (var, maybe_mask) in masks {
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
        Stmt::ExprStmt(Expr::Call(callee, _)) => matches!(
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
/// when guards hold), walking through `local_defs` up to depth 4.
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

/// Resolves `base + iv >>> 0` or `(base + iv | 0) - sub >>> 0` (exact fflate LZ77 byte-match shape).
enum BaseIvShape {
    /// `base + iv >>> 0`
    Add { base: String },
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
        let node = match node {
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

fn derive_check_guards(
    idx: &Expr,
    len: &Expr,
    counter: Option<&CounterInfo>,
    loop_masked: &HashMap<String, Expr>,
    local_defs: &HashMap<String, Expr>,
    assigned_in_loop: &HashSet<String>,
) -> Option<Vec<Expr>> {
    // Resolve `len` through `local_defs` (e.g. `G = $hl_0` after LICM hoisted `r.len`), then
    // verify `len` is a constant u32 or a loop-invariant identifier.
    let resolved_len: Expr = match len {
        Expr::Ident(id) if assigned_in_loop.contains(id) => local_defs.get(id)?.clone(),
        other => other.clone(),
    };
    let len = &resolved_len;
    let len_inv = match len {
        Expr::Ident(id) => !assigned_in_loop.contains(id),
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
    let mask_opt = extract_bitand_mask(idx, assigned_in_loop).or_else(|| match idx {
        Expr::Ident(id) => loop_masked.get(id).cloned().or_else(|| {
            local_defs
                .get(id)
                .and_then(|d| extract_bitand_mask(d, assigned_in_loop))
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

    let c = counter?;

    // 3. Affine in `iv`: `scale * iv + offset`.
    if let Some((scale, offset)) = resolve_affine_iv(idx, &c.iv, local_defs, 0) {
        if scale >= 1 {
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
                    if max_val < i64::from(lc) {
                        return Some(guards);
                    }
                    return None;
                }
                guards.push(Expr::Binary(
                    BinOp::Lt,
                    Box::new(Expr::Num(max_val as f64)),
                    Box::new(len.clone()),
                ));
                return Some(guards);
            } else if scale == 1 && offset <= 0 {
                // `iv + offset < bound <= len`
                guards.push(Expr::Binary(
                    BinOp::Le,
                    Box::new(c.bound.clone()),
                    Box::new(len.clone()),
                ));
                return Some(guards);
            }
        }
    }

    // 4. `base + iv >>> 0` or `(base + iv | 0) - sub >>> 0`.
    if let Some(shape) = resolve_base_iv_shape(idx, &c.iv, local_defs, assigned_in_loop) {
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
                return Some(guards);
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
                return Some(guards);
            }
        }
    }

    None
}

fn simplify_guards(all_guards: Vec<Expr>) -> Vec<Expr> {
    // First pass: record the strictest constant LHS for `K < rhs` (keep max K) and `C <= rhs`
    // (keep max C).
    let mut max_lt: Vec<(Expr, u32)> = Vec::new();
    let mut max_le: Vec<(Expr, u32)> = Vec::new();
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
            }
            _ => {}
        }
    }

    let mut deduped: Vec<Expr> = Vec::new();
    for g in all_guards {
        if let Expr::Binary(BinOp::Lt, l, r) = &g {
            if let Some(k) = exact_u32(l) {
                if let Some((_, max_k)) = max_lt.iter().find(|(e, _)| e == r.as_ref()) {
                    if k < *max_k {
                        continue;
                    }
                }
            }
        } else if let Expr::Binary(BinOp::Le, l, r) = &g {
            if let Some(c) = exact_u32(l) {
                if let Some((_, max_c)) = max_le.iter().find(|(e, _)| e == r.as_ref()) {
                    if c < *max_c {
                        continue;
                    }
                }
            }
        }
        if !deduped.contains(&g) {
            deduped.push(g);
        }
    }
    deduped
}

fn try_version_loop_at(
    enclosing: &[Stmt],
    loop_idx: usize,
    pre_loop_masked: &HashMap<String, Expr>,
) -> Option<Stmt> {
    if !is_loop_stmt(&enclosing[loop_idx]) {
        return None;
    }
    let body = extract_loop_body(&enclosing[loop_idx])?;
    if count_stmts(body) > 180 {
        return None;
    }

    let mut assigned_in_loop = HashSet::new();
    collect_written_idents_stmts(body, &mut assigned_in_loop, true);
    let counter = detect_loop_counter(enclosing, loop_idx, &assigned_in_loop);
    let loop_masked = detect_loop_masked_vars(
        enclosing,
        loop_idx,
        body,
        &assigned_in_loop,
        pre_loop_masked,
    );

    // Clone the loop and strip all bounds checks we can prove with invariant pre-loop guards.
    let mut fast_loop = enclosing[loop_idx].clone();
    let mut all_guards: Vec<Expr> = Vec::new();
    let mut removed = 0usize;

    let fast_body = extract_loop_body_mut(&mut fast_loop)?;
    strip_provable_checks(
        fast_body,
        counter.as_ref(),
        &loop_masked,
        &assigned_in_loop,
        &mut all_guards,
        &mut removed,
    );

    if removed == 0 {
        return None;
    }

    let mut deduped = simplify_guards(all_guards);
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

fn strip_provable_checks(
    stmts: &mut Vec<Stmt>,
    counter: Option<&CounterInfo>,
    loop_masked: &HashMap<String, Expr>,
    assigned_in_loop: &HashSet<String>,
    all_guards: &mut Vec<Expr>,
    removed: &mut usize,
) {
    let mut local_defs: HashMap<String, Expr> = HashMap::new();
    let mut i = 0;
    while i < stmts.len() {
        if let Some((idx, len)) = is_panic_bounds_check_if(&stmts[i], &local_defs) {
            if let Some(guards) = derive_check_guards(
                &idx,
                &len,
                counter,
                loop_masked,
                &local_defs,
                assigned_in_loop,
            ) {
                all_guards.extend(guards);
                stmts.remove(i);
                *removed += 1;
                continue;
            }
        }
        match &mut stmts[i] {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if let Expr::Ident(id) = lhs.as_ref() {
                    let id = id.clone();
                    // Invalidate any cached local def that references `id`.
                    local_defs.retain(|k, v| k != &id && !expr_reads_ident(v, &id));
                    if !expr_has_side_effect(rhs) && !expr_reads_ident(rhs, &id) {
                        local_defs.insert(id, rhs.as_ref().clone());
                    }
                }
            }
            Stmt::If(_, a, b) => {
                // Only recurse into `if` branches (do NOT cross into nested loops because their
                // induction variables and `assigned_in_loop` differ; inner loops were already
                // processed bottom-up!).
                if !stmts_definitely_diverge(a) {
                    let mut a_defs = local_defs.clone();
                    strip_provable_checks_with_defs(
                        a,
                        &mut a_defs,
                        counter,
                        loop_masked,
                        assigned_in_loop,
                        all_guards,
                        removed,
                    );
                }
                if let Some(b) = b {
                    if !stmts_definitely_diverge(b) {
                        let mut b_defs = local_defs.clone();
                        strip_provable_checks_with_defs(
                            b,
                            &mut b_defs,
                            counter,
                            loop_masked,
                            assigned_in_loop,
                            all_guards,
                            removed,
                        );
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
            Stmt::Block(b) => {
                strip_provable_checks_with_defs(
                    b,
                    &mut local_defs,
                    counter,
                    loop_masked,
                    assigned_in_loop,
                    all_guards,
                    removed,
                );
            }
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(
                    std::slice::from_ref(inner.as_ref()),
                    &mut written,
                    false,
                );
                if let Stmt::Block(b) = inner.as_mut() {
                    let mut b_defs = local_defs.clone();
                    strip_provable_checks_with_defs(
                        b,
                        &mut b_defs,
                        counter,
                        loop_masked,
                        assigned_in_loop,
                        all_guards,
                        removed,
                    );
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

fn strip_provable_checks_with_defs(
    stmts: &mut Vec<Stmt>,
    local_defs: &mut HashMap<String, Expr>,
    counter: Option<&CounterInfo>,
    loop_masked: &HashMap<String, Expr>,
    assigned_in_loop: &HashSet<String>,
    all_guards: &mut Vec<Expr>,
    removed: &mut usize,
) {
    let mut i = 0;
    while i < stmts.len() {
        if let Some((idx, len)) = is_panic_bounds_check_if(&stmts[i], local_defs) {
            if let Some(guards) = derive_check_guards(
                &idx,
                &len,
                counter,
                loop_masked,
                local_defs,
                assigned_in_loop,
            ) {
                all_guards.extend(guards);
                stmts.remove(i);
                *removed += 1;
                continue;
            }
        }
        match &mut stmts[i] {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                if let Expr::Ident(id) = lhs.as_ref() {
                    let id = id.clone();
                    local_defs.retain(|k, v| k != &id && !expr_reads_ident(v, &id));
                    if !expr_has_side_effect(rhs) && !expr_reads_ident(rhs, &id) {
                        local_defs.insert(id, rhs.as_ref().clone());
                    }
                }
            }
            Stmt::If(_, a, b) => {
                if !stmts_definitely_diverge(a) {
                    let mut a_defs = local_defs.clone();
                    strip_provable_checks_with_defs(
                        a,
                        &mut a_defs,
                        counter,
                        loop_masked,
                        assigned_in_loop,
                        all_guards,
                        removed,
                    );
                }
                if let Some(b) = b {
                    if !stmts_definitely_diverge(b) {
                        let mut b_defs = local_defs.clone();
                        strip_provable_checks_with_defs(
                            b,
                            &mut b_defs,
                            counter,
                            loop_masked,
                            assigned_in_loop,
                            all_guards,
                            removed,
                        );
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
            Stmt::Block(b) => {
                strip_provable_checks_with_defs(
                    b,
                    local_defs,
                    counter,
                    loop_masked,
                    assigned_in_loop,
                    all_guards,
                    removed,
                );
            }
            Stmt::Labeled(_, inner) => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(
                    std::slice::from_ref(inner.as_ref()),
                    &mut written,
                    false,
                );
                if let Stmt::Block(b) = inner.as_mut() {
                    let mut b_defs = local_defs.clone();
                    strip_provable_checks_with_defs(
                        b,
                        &mut b_defs,
                        counter,
                        loop_masked,
                        assigned_in_loop,
                        all_guards,
                        removed,
                    );
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

// =================================================================================================
// Shared AST Helpers
// =================================================================================================

fn count_stmts(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    for s in stmts {
        n += 1;
        match s {
            Stmt::If(_, a, b) => {
                n += count_stmts(a);
                if let Some(b) = b {
                    n += count_stmts(b);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    n += count_stmts(&c.body);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => n += count_stmts(b),
            Stmt::Labeled(_, inner) => n += count_stmts(std::slice::from_ref(inner.as_ref())),
            Stmt::TryCatch { body, handler, .. } => {
                n += count_stmts(body) + count_stmts(handler);
            }
            _ => {}
        }
    }
    n
}

/// Returns `true` if `expr` contains a nested arrow/function that may mutate outer state or
/// execute arbitrary side effects. Pure expression-bodied arrows (like `m => ({ hi: m.hi, lo: m.lo })`
/// emitted for array clones in `compress512`) are read-only and do not block outer loop passes.
fn expr_has_mutating_nested_fn(expr: &Expr) -> bool {
    match expr {
        Expr::Arrow(_, ArrowBody::Expr(body)) => {
            expr_has_side_effect(body) || expr_has_mutating_nested_fn(body)
        }
        Expr::Arrow(_, ArrowBody::Block(_)) => true,
        Expr::Unary(_, e) | Expr::Member(e, _) => expr_has_mutating_nested_fn(e),
        Expr::Binary(_, l, r)
        | Expr::Assign(l, r)
        | Expr::CompoundAssign(_, l, r)
        | Expr::Index(l, r) => expr_has_mutating_nested_fn(l) || expr_has_mutating_nested_fn(r),
        Expr::Cond(c, t, e) => {
            expr_has_mutating_nested_fn(c)
                || expr_has_mutating_nested_fn(t)
                || expr_has_mutating_nested_fn(e)
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            expr_has_mutating_nested_fn(c) || args.iter().any(expr_has_mutating_nested_fn)
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().any(expr_has_mutating_nested_fn),
        Expr::Object(entries) => entries.iter().any(|(_, v)| expr_has_mutating_nested_fn(v)),
        _ => false,
    }
}

fn stmts_have_mutating_nested_fn(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::FunctionDecl { .. } => true,
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
            expr_has_mutating_nested_fn(e)
        }
        Stmt::Let(_, Some(e)) => expr_has_mutating_nested_fn(e),
        Stmt::Lets(decls) => decls
            .iter()
            .any(|(_, init)| init.as_ref().is_some_and(expr_has_mutating_nested_fn)),
        Stmt::If(c, a, b) => {
            expr_has_mutating_nested_fn(c)
                || stmts_have_mutating_nested_fn(a)
                || b.as_ref().is_some_and(|b| stmts_have_mutating_nested_fn(b))
        }
        Stmt::Switch(d, cases) => {
            expr_has_mutating_nested_fn(d)
                || cases.iter().any(|c| stmts_have_mutating_nested_fn(&c.body))
        }
        Stmt::Block(b) | Stmt::LoopForever(_, b) => stmts_have_mutating_nested_fn(b),
        Stmt::While(c, b) => expr_has_mutating_nested_fn(c) || stmts_have_mutating_nested_fn(b),
        Stmt::Labeled(_, inner) => {
            stmts_have_mutating_nested_fn(std::slice::from_ref(inner.as_ref()))
        }
        Stmt::TryCatch { body, handler, .. } => {
            stmts_have_mutating_nested_fn(body) || stmts_have_mutating_nested_fn(handler)
        }
        _ => false,
    })
}

fn expr_has_side_effect(expr: &Expr) -> bool {
    match expr {
        Expr::Assign(..) | Expr::CompoundAssign(..) | Expr::New(..) | Expr::Raw(_) => true,
        Expr::Call(callee, args) => {
            let is_pure_math = matches!(
                callee.as_ref(),
                Expr::Member(obj, prop)
                    if matches!(obj.as_ref(), Expr::Ident(m) if m == "Math")
                        && matches!(prop.as_str(), "imul" | "clz32" | "abs" | "min" | "max")
            );
            !is_pure_math || args.iter().any(expr_has_side_effect)
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => expr_has_side_effect(e),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            expr_has_side_effect(l) || expr_has_side_effect(r)
        }
        Expr::Cond(c, t, e) => {
            expr_has_side_effect(c) || expr_has_side_effect(t) || expr_has_side_effect(e)
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().any(expr_has_side_effect),
        Expr::Object(entries) => entries.iter().any(|(_, v)| expr_has_side_effect(v)),
        _ => false,
    }
}

fn expr_has_non_panic_call(expr: &Expr) -> bool {
    match expr {
        Expr::New(..) | Expr::Raw(_) => true,
        Expr::Call(callee, args) => {
            let is_pure_or_panic = match callee.as_ref() {
                Expr::Ident(n) if n.starts_with("core$panicking$") => true,
                Expr::Member(obj, prop)
                    if matches!(obj.as_ref(), Expr::Ident(m) if m == "Math")
                        && matches!(prop.as_str(), "imul" | "clz32" | "abs" | "min" | "max") =>
                {
                    true
                }
                _ => false,
            };
            if matches!(callee.as_ref(), Expr::Ident(n) if n.starts_with("core$panicking$")) {
                return false;
            }
            !is_pure_or_panic || args.iter().any(expr_has_non_panic_call)
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => expr_has_non_panic_call(e),
        Expr::Binary(_, l, r)
        | Expr::Assign(l, r)
        | Expr::CompoundAssign(_, l, r)
        | Expr::Index(l, r) => expr_has_non_panic_call(l) || expr_has_non_panic_call(r),
        Expr::Cond(c, t, e) => {
            expr_has_non_panic_call(c) || expr_has_non_panic_call(t) || expr_has_non_panic_call(e)
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().any(expr_has_non_panic_call),
        Expr::Object(entries) => entries.iter().any(|(_, v)| expr_has_non_panic_call(v)),
        _ => false,
    }
}

fn stmts_have_non_panic_call(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
            expr_has_non_panic_call(e)
        }
        Stmt::Let(_, Some(e)) => expr_has_non_panic_call(e),
        Stmt::Lets(decls) => decls
            .iter()
            .any(|(_, init)| init.as_ref().is_some_and(expr_has_non_panic_call)),
        Stmt::If(c, a, b) => {
            expr_has_non_panic_call(c)
                || (!stmts_definitely_diverge(a) && stmts_have_non_panic_call(a))
                || b.as_ref()
                    .is_some_and(|b| !stmts_definitely_diverge(b) && stmts_have_non_panic_call(b))
        }
        Stmt::Switch(d, cases) => {
            expr_has_non_panic_call(d) || cases.iter().any(|c| stmts_have_non_panic_call(&c.body))
        }
        Stmt::Block(b) | Stmt::LoopForever(_, b) => stmts_have_non_panic_call(b),
        Stmt::While(c, b) => expr_has_non_panic_call(c) || stmts_have_non_panic_call(b),
        Stmt::Labeled(_, inner) => stmts_have_non_panic_call(std::slice::from_ref(inner.as_ref())),
        Stmt::TryCatch { body, handler, .. } => {
            stmts_have_non_panic_call(body) || stmts_have_non_panic_call(handler)
        }
        _ => false,
    })
}

fn stmt_has_panic_call(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::ExprStmt(Expr::Call(callee, _)) => {
            matches!(callee.as_ref(), Expr::Ident(n) if n.starts_with("core$panicking$"))
        }
        Stmt::If(_, a, b) => {
            a.iter().any(stmt_has_panic_call)
                || b.as_ref()
                    .is_some_and(|b| b.iter().any(stmt_has_panic_call))
        }
        Stmt::Switch(_, cases) => cases.iter().any(|c| c.body.iter().any(stmt_has_panic_call)),
        Stmt::Block(b) | Stmt::LoopForever(_, b) | Stmt::While(_, b) => {
            b.iter().any(stmt_has_panic_call)
        }
        Stmt::Labeled(_, inner) => stmt_has_panic_call(inner),
        Stmt::TryCatch { body, handler, .. } => {
            body.iter().any(stmt_has_panic_call) || handler.iter().any(stmt_has_panic_call)
        }
        _ => false,
    }
}

fn stmts_have_control_flow(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Return(_)
        | Stmt::Throw(_)
        | Stmt::LoopForever(..)
        | Stmt::While(..)
        | Stmt::Switch(..)
        | Stmt::TryCatch { .. } => true,
        Stmt::If(_, a, b) => {
            (!stmts_definitely_diverge(a) && stmts_have_control_flow(a))
                || b.as_ref()
                    .is_some_and(|b| !stmts_definitely_diverge(b) && stmts_have_control_flow(b))
        }
        Stmt::Block(b) => stmts_have_control_flow(b),
        Stmt::Labeled(_, inner) => stmts_have_control_flow(std::slice::from_ref(inner.as_ref())),
        _ => false,
    })
}

fn stmts_have_var_decl(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Let(..) | Stmt::Const(..) | Stmt::Lets(..) | Stmt::FunctionDecl { .. } => true,
        Stmt::If(_, a, b) => {
            stmts_have_var_decl(a) || b.as_ref().is_some_and(|b| stmts_have_var_decl(b))
        }
        Stmt::Block(b) => stmts_have_var_decl(b),
        Stmt::Labeled(_, inner) => stmts_have_var_decl(std::slice::from_ref(inner.as_ref())),
        _ => false,
    })
}

fn collect_written_idents_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            if let Expr::Ident(id) = lhs.as_ref() {
                out.insert(id.clone());
            } else {
                collect_written_idents_expr(lhs, out);
            }
            collect_written_idents_expr(rhs, out);
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => collect_written_idents_expr(e, out),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            collect_written_idents_expr(l, out);
            collect_written_idents_expr(r, out);
        }
        Expr::Cond(c, t, e) => {
            collect_written_idents_expr(c, out);
            collect_written_idents_expr(t, out);
            collect_written_idents_expr(e, out);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            collect_written_idents_expr(c, out);
            for a in args {
                collect_written_idents_expr(a, out);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                collect_written_idents_expr(x, out);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                collect_written_idents_expr(v, out);
            }
        }
        _ => {}
    }
}

fn collect_written_idents_stmts(
    stmts: &[Stmt],
    out: &mut HashSet<String>,
    ignore_diverging_branches: bool,
) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                collect_written_idents_expr(e, out);
            }
            Stmt::Let(n, init) => {
                out.insert(n.clone());
                if let Some(e) = init {
                    collect_written_idents_expr(e, out);
                }
            }
            Stmt::Const(n, e) => {
                out.insert(n.clone());
                collect_written_idents_expr(e, out);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    out.insert(n.clone());
                    if let Some(e) = init {
                        collect_written_idents_expr(e, out);
                    }
                }
            }
            Stmt::If(c, a, b) => {
                collect_written_idents_expr(c, out);
                if !(ignore_diverging_branches && stmts_definitely_diverge(a)) {
                    collect_written_idents_stmts(a, out, ignore_diverging_branches);
                }
                if let Some(b) = b {
                    if !(ignore_diverging_branches && stmts_definitely_diverge(b)) {
                        collect_written_idents_stmts(b, out, ignore_diverging_branches);
                    }
                }
            }
            Stmt::Switch(d, cases) => {
                collect_written_idents_expr(d, out);
                for c in cases {
                    collect_written_idents_stmts(&c.body, out, ignore_diverging_branches);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) => {
                collect_written_idents_stmts(b, out, ignore_diverging_branches);
            }
            Stmt::While(c, b) => {
                collect_written_idents_expr(c, out);
                collect_written_idents_stmts(b, out, ignore_diverging_branches);
            }
            Stmt::Labeled(_, inner) => {
                collect_written_idents_stmts(
                    std::slice::from_ref(inner.as_ref()),
                    out,
                    ignore_diverging_branches,
                );
            }
            Stmt::TryCatch { body, handler, .. } => {
                collect_written_idents_stmts(body, out, false);
                collect_written_idents_stmts(handler, out, false);
            }
            _ => {}
        }
    }
}

fn expr_writes_ident(expr: &Expr, target: &str) -> bool {
    let mut w = HashSet::new();
    collect_written_idents_expr(expr, &mut w);
    w.contains(target)
}

fn expr_reads_ident(expr: &Expr, target: &str) -> bool {
    match expr {
        Expr::Ident(id) => id == target,
        Expr::Assign(lhs, rhs) => {
            let l_read = !matches!(lhs.as_ref(), Expr::Ident(_)) && expr_reads_ident(lhs, target);
            l_read || expr_reads_ident(rhs, target)
        }
        Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            expr_reads_ident(l, target) || expr_reads_ident(r, target)
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => expr_reads_ident(e, target),
        Expr::Cond(c, t, e) => {
            expr_reads_ident(c, target)
                || expr_reads_ident(t, target)
                || expr_reads_ident(e, target)
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            expr_reads_ident(c, target) || args.iter().any(|a| expr_reads_ident(a, target))
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().any(|x| expr_reads_ident(x, target)),
        Expr::Object(entries) => entries.iter().any(|(_, v)| expr_reads_ident(v, target)),
        Expr::Arrow(params, ArrowBody::Expr(body)) => {
            !params.iter().any(|p| p == target) && expr_reads_ident(body, target)
        }
        _ => false,
    }
}

fn stmts_read_ident(stmts: &[Stmt], target: &str) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
            expr_reads_ident(e, target)
        }
        Stmt::Let(_, Some(e)) => expr_reads_ident(e, target),
        Stmt::Lets(decls) => decls
            .iter()
            .any(|(_, init)| init.as_ref().is_some_and(|e| expr_reads_ident(e, target))),
        Stmt::If(c, a, b) => {
            expr_reads_ident(c, target)
                || stmts_read_ident(a, target)
                || b.as_ref().is_some_and(|b| stmts_read_ident(b, target))
        }
        Stmt::Switch(d, cases) => {
            expr_reads_ident(d, target) || cases.iter().any(|c| stmts_read_ident(&c.body, target))
        }
        Stmt::Block(b) | Stmt::LoopForever(_, b) => stmts_read_ident(b, target),
        Stmt::While(c, b) => expr_reads_ident(c, target) || stmts_read_ident(b, target),
        Stmt::Labeled(_, inner) => stmts_read_ident(std::slice::from_ref(inner.as_ref()), target),
        Stmt::TryCatch { body, handler, .. } => {
            stmts_read_ident(body, target) || stmts_read_ident(handler, target)
        }
        _ => false,
    })
}

fn subst_ident_in_expr(expr: &mut Expr, from: &str, to: &Expr) {
    match expr {
        Expr::Ident(id) if id == from => *expr = to.clone(),
        Expr::Assign(lhs, rhs) => {
            if !matches!(lhs.as_ref(), Expr::Ident(_)) {
                subst_ident_in_expr(lhs, from, to);
            }
            subst_ident_in_expr(rhs, from, to);
        }
        Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            subst_ident_in_expr(l, from, to);
            subst_ident_in_expr(r, from, to);
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => subst_ident_in_expr(e, from, to),
        Expr::Cond(c, t, e) => {
            subst_ident_in_expr(c, from, to);
            subst_ident_in_expr(t, from, to);
            subst_ident_in_expr(e, from, to);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            subst_ident_in_expr(c, from, to);
            for a in args {
                subst_ident_in_expr(a, from, to);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                subst_ident_in_expr(x, from, to);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                subst_ident_in_expr(v, from, to);
            }
        }
        _ => {}
    }
}

fn subst_ident_in_stmt(stmt: &mut Stmt, from: &str, to: &Expr) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
            subst_ident_in_expr(e, from, to);
        }
        Stmt::Let(_, Some(e)) => subst_ident_in_expr(e, from, to),
        Stmt::Lets(decls) => {
            for (_, init) in decls {
                if let Some(e) = init {
                    subst_ident_in_expr(e, from, to);
                }
            }
        }
        Stmt::If(c, a, b) => {
            subst_ident_in_expr(c, from, to);
            for s in a {
                subst_ident_in_stmt(s, from, to);
            }
            if let Some(b) = b {
                for s in b {
                    subst_ident_in_stmt(s, from, to);
                }
            }
        }
        Stmt::Block(b) => {
            for s in b {
                subst_ident_in_stmt(s, from, to);
            }
        }
        Stmt::Labeled(_, inner) => subst_ident_in_stmt(inner, from, to),
        _ => {}
    }
}

fn count_rw_expr(
    expr: &Expr,
    reads: &mut HashMap<String, usize>,
    writes: &mut HashMap<String, usize>,
) {
    match expr {
        Expr::Ident(id) => *reads.entry(id.clone()).or_default() += 1,
        Expr::Assign(lhs, rhs) => {
            if let Expr::Ident(id) = lhs.as_ref() {
                *writes.entry(id.clone()).or_default() += 1;
            } else {
                count_rw_expr(lhs, reads, writes);
            }
            count_rw_expr(rhs, reads, writes);
        }
        Expr::CompoundAssign(_, lhs, rhs) => {
            if let Expr::Ident(id) = lhs.as_ref() {
                *reads.entry(id.clone()).or_default() += 1;
                *writes.entry(id.clone()).or_default() += 1;
            } else {
                count_rw_expr(lhs, reads, writes);
            }
            count_rw_expr(rhs, reads, writes);
        }
        Expr::Unary(_, e) | Expr::Member(e, _) => count_rw_expr(e, reads, writes),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => {
            count_rw_expr(l, reads, writes);
            count_rw_expr(r, reads, writes);
        }
        Expr::Cond(c, t, e) => {
            count_rw_expr(c, reads, writes);
            count_rw_expr(t, reads, writes);
            count_rw_expr(e, reads, writes);
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            count_rw_expr(c, reads, writes);
            for a in args {
                count_rw_expr(a, reads, writes);
            }
        }
        Expr::Array(items) | Expr::Seq(items) => {
            for x in items {
                count_rw_expr(x, reads, writes);
            }
        }
        Expr::Object(entries) => {
            for (_, v) in entries {
                count_rw_expr(v, reads, writes);
            }
        }
        Expr::Arrow(_, body) => match body {
            ArrowBody::Expr(e) => count_rw_expr(e, reads, writes),
            ArrowBody::Block(b) => count_rw_stmts(b, reads, writes),
        },
        _ => {}
    }
}

fn count_rw_stmts(
    stmts: &[Stmt],
    reads: &mut HashMap<String, usize>,
    writes: &mut HashMap<String, usize>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                count_rw_expr(e, reads, writes);
            }
            Stmt::Let(n, Some(e)) | Stmt::Const(n, e) => {
                *writes.entry(n.clone()).or_default() += 1;
                count_rw_expr(e, reads, writes);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(e) = init {
                        *writes.entry(n.clone()).or_default() += 1;
                        count_rw_expr(e, reads, writes);
                    }
                }
            }
            Stmt::If(c, a, b) => {
                count_rw_expr(c, reads, writes);
                count_rw_stmts(a, reads, writes);
                if let Some(b) = b {
                    count_rw_stmts(b, reads, writes);
                }
            }
            Stmt::Switch(d, cases) => {
                count_rw_expr(d, reads, writes);
                for c in cases {
                    count_rw_stmts(&c.body, reads, writes);
                }
            }
            Stmt::Block(b) | Stmt::LoopForever(_, b) => count_rw_stmts(b, reads, writes),
            Stmt::While(c, b) => {
                count_rw_expr(c, reads, writes);
                count_rw_stmts(b, reads, writes);
            }
            Stmt::Labeled(_, inner) => {
                count_rw_stmts(std::slice::from_ref(inner.as_ref()), reads, writes);
            }
            Stmt::TryCatch { body, handler, .. } => {
                count_rw_stmts(body, reads, writes);
                count_rw_stmts(handler, reads, writes);
            }
            Stmt::FunctionDecl { body, .. } => count_rw_stmts(body, reads, writes),
            _ => {}
        }
    }
}

// =================================================================================================
// Unit Tests
// =================================================================================================

#[cfg(test)]
mod tests {
    use super::super::post_opt::run_speed_post_opts;
    use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp};

    #[test]
    fn folds_guarded_f2i_clamp_to_bitor_zero() {
        let mut decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["n".to_owned()],
            body: vec![
                jsast::lets(vec![("S".to_owned(), None)]),
                Stmt::If(
                    Expr::Binary(
                        BinOp::Ge,
                        Box::new(jsast::id("n")),
                        Box::new(Expr::Num(-2147483648.0)),
                    ),
                    vec![Stmt::If(
                        Expr::Binary(
                            BinOp::Le,
                            Box::new(jsast::id("n")),
                            Box::new(Expr::Num(2147483647.0)),
                        ),
                        vec![jsast::assign_stmt(
                            jsast::id("S"),
                            jsast::cond(
                                Expr::Binary(
                                    BinOp::Le,
                                    Box::new(jsast::id("n")),
                                    Box::new(Expr::Num(-2147483648.0)),
                                ),
                                Expr::Num(-2147483648.0),
                                jsast::cond(
                                    Expr::Binary(
                                        BinOp::Ge,
                                        Box::new(jsast::id("n")),
                                        Box::new(Expr::Num(2147483647.0)),
                                    ),
                                    Expr::Num(2147483647.0),
                                    Expr::Binary(
                                        BinOp::BitOr,
                                        Box::new(jsast::id("n")),
                                        Box::new(Expr::Num(0.0)),
                                    ),
                                ),
                            ),
                        )],
                        None,
                    )],
                    None,
                ),
                Stmt::Return(Some(jsast::id("S"))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let s = jsast::stmt_to_string(&decl);
        assert!(s.contains("S = n | 0;"), "expected S = n | 0: {s}");
        assert!(!s.contains("?"), "expected clamp ternary gone: {s}");
    }

    #[test]
    fn versions_fflate_match_loop_and_hoists_slice_props() {
        let mut decl = Stmt::FunctionDecl {
            name: "match_loop".to_owned(),
            params: vec![
                "a".to_owned(),
                "g".to_owned(),
                "l".to_owned(),
                "bh".to_owned(),
            ],
            body: vec![
                jsast::lets(vec![
                    ("h".to_owned(), None),
                    ("j".to_owned(), None),
                    ("ak".to_owned(), None),
                    ("al".to_owned(), None),
                    ("br".to_owned(), None),
                ]),
                jsast::assign_stmt(jsast::id("h"), jsast::member(jsast::id("a"), "len")),
                jsast::assign_stmt(jsast::id("j"), Expr::Num(0.0)),
                Stmt::While(
                    Expr::Binary(
                        BinOp::Lt,
                        Box::new(jsast::id("j")),
                        Box::new(jsast::id("bh")),
                    ),
                    vec![
                        jsast::assign_stmt(
                            jsast::id("ak"),
                            Expr::Binary(
                                BinOp::UShr,
                                Box::new(Expr::Binary(
                                    BinOp::Add,
                                    Box::new(jsast::id("g")),
                                    Box::new(jsast::id("j")),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                        ),
                        Stmt::If(
                            Expr::Unary(
                                UnOp::Not,
                                Box::new(Expr::Binary(
                                    BinOp::Lt,
                                    Box::new(jsast::id("ak")),
                                    Box::new(jsast::id("h")),
                                )),
                            ),
                            vec![Stmt::ExprStmt(jsast::call(
                                jsast::id("core$panicking$panic_bounds_check$h1"),
                                vec![jsast::id("ak"), jsast::id("h")],
                            ))],
                            None,
                        ),
                        jsast::assign_stmt(
                            jsast::id("br"),
                            jsast::index(
                                jsast::member(jsast::id("a"), "buf"),
                                Expr::Binary(
                                    BinOp::Add,
                                    Box::new(jsast::member(jsast::id("a"), "off")),
                                    Box::new(jsast::id("ak")),
                                ),
                            ),
                        ),
                        jsast::assign_stmt(
                            jsast::id("al"),
                            Expr::Binary(
                                BinOp::UShr,
                                Box::new(Expr::Binary(
                                    BinOp::Sub,
                                    Box::new(Expr::Binary(
                                        BinOp::BitOr,
                                        Box::new(Expr::Binary(
                                            BinOp::Add,
                                            Box::new(jsast::id("g")),
                                            Box::new(jsast::id("j")),
                                        )),
                                        Box::new(Expr::Num(0.0)),
                                    )),
                                    Box::new(jsast::id("l")),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                        ),
                        Stmt::If(
                            Expr::Unary(
                                UnOp::Not,
                                Box::new(Expr::Binary(
                                    BinOp::Lt,
                                    Box::new(jsast::id("al")),
                                    Box::new(jsast::id("h")),
                                )),
                            ),
                            vec![Stmt::ExprStmt(jsast::call(
                                jsast::id("core$panicking$panic_bounds_check$h1"),
                                vec![jsast::id("al"), jsast::id("h")],
                            ))],
                            None,
                        ),
                        jsast::assign_stmt(
                            jsast::id("j"),
                            Expr::Binary(
                                BinOp::UShr,
                                Box::new(Expr::Binary(
                                    BinOp::Add,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(1.0)),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                        ),
                    ],
                ),
                Stmt::Return(Some(jsast::id("j"))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let s = jsast::stmt_to_string(&decl);
        assert!(s.contains("$hb_0 = a.buf;"), "expected a.buf hoisted: {s}");
        assert!(s.contains("$ho_0 = a.off;"), "expected a.off hoisted: {s}");
        assert_eq!(
            s.matches("while (j < bh)").count(),
            2,
            "expected fast+slow loop versioning: {s}"
        );
        assert_eq!(
            s.matches("panic_bounds_check").count(),
            2,
            "expected panic checks only in fallback branch: {s}"
        );
    }

    #[test]
    fn unrolls_small_const_loop_and_folds_indices() {
        let mut decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["a".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("m".to_owned(), None),
                    ("next".to_owned(), None),
                    ("s".to_owned(), None),
                ]),
                jsast::assign_stmt(jsast::id("m"), Expr::Num(0.0)),
                Stmt::LoopForever(
                    None,
                    vec![Stmt::If(
                        Expr::Binary(
                            BinOp::Lt,
                            Box::new(jsast::id("m")),
                            Box::new(Expr::Num(5.0)),
                        ),
                        vec![
                            jsast::assign_stmt(
                                jsast::id("next"),
                                Expr::Binary(
                                    BinOp::UShr,
                                    Box::new(Expr::Binary(
                                        BinOp::Add,
                                        Box::new(jsast::id("m")),
                                        Box::new(Expr::Num(1.0)),
                                    )),
                                    Box::new(Expr::Num(0.0)),
                                ),
                            ),
                            jsast::assign_stmt(
                                jsast::id("s"),
                                jsast::method_call(
                                    jsast::id("Math"),
                                    "imul",
                                    vec![Expr::Num(2.0), jsast::id("m")],
                                ),
                            ),
                            jsast::assign_stmt(
                                jsast::index(jsast::id("a"), jsast::id("s")),
                                jsast::id("m"),
                            ),
                            jsast::assign_stmt(jsast::id("m"), jsast::id("next")),
                        ],
                        Some(vec![Stmt::Break(None)]),
                    )],
                ),
            ],
        };
        run_speed_post_opts(&mut decl);
        let s = jsast::stmt_to_string(&decl);
        assert!(!s.contains("for (;;)"), "expected loop unrolled: {s}");
        assert!(!s.contains("Math.imul"), "expected Math.imul folded: {s}");
        assert!(s.contains("a[8] = 4;"), "expected a[8] = 4: {s}");
    }

    #[test]
    fn elides_shallow_obj_map_clones_and_versions_through_pure_arrow() {
        let mut decl = Stmt::FunctionDecl {
            name: "compress_like".to_owned(),
            params: vec![
                "g".to_owned(),
                "i".to_owned(),
                "p".to_owned(),
                "q".to_owned(),
            ],
            body: vec![
                jsast::lets(vec![
                    ("l".to_owned(), None),
                    ("b9".to_owned(), None),
                    ("bp".to_owned(), None),
                ]),
                jsast::assign_stmt(jsast::id("l"), Expr::Num(16.0)),
                Stmt::While(
                    Expr::Binary(
                        BinOp::Lt,
                        Box::new(jsast::id("l")),
                        Box::new(Expr::Num(64.0)),
                    ),
                    vec![
                        Stmt::If(
                            Expr::Unary(
                                UnOp::Not,
                                Box::new(Expr::Binary(
                                    BinOp::Lt,
                                    Box::new(Expr::Binary(
                                        BinOp::Sub,
                                        Box::new(jsast::id("l")),
                                        Box::new(Expr::Num(15.0)),
                                    )),
                                    Box::new(jsast::id("i")),
                                )),
                            ),
                            vec![Stmt::ExprStmt(jsast::call(
                                jsast::id("core$panicking$panic_bounds_check$h1"),
                                vec![jsast::id("l"), jsast::id("i")],
                            ))],
                            None,
                        ),
                        Stmt::If(
                            Expr::Unary(
                                UnOp::Not,
                                Box::new(Expr::Binary(
                                    BinOp::Lt,
                                    Box::new(jsast::id("l")),
                                    Box::new(jsast::id("i")),
                                )),
                            ),
                            vec![Stmt::ExprStmt(jsast::call(
                                jsast::id("core$panicking$panic_bounds_check$h1"),
                                vec![jsast::id("l"), jsast::id("i")],
                            ))],
                            None,
                        ),
                        jsast::assign_stmt(
                            jsast::id("l"),
                            Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("l")),
                                Box::new(Expr::Num(1.0)),
                            ),
                        ),
                    ],
                ),
                jsast::assign_stmt(
                    jsast::id("b9"),
                    Expr::Array(vec![Expr::Object(vec![
                        ("hi".to_owned(), jsast::id("p")),
                        ("lo".to_owned(), jsast::id("q")),
                    ])]),
                ),
                jsast::assign_stmt(
                    jsast::id("bp"),
                    jsast::method_call(
                        jsast::id("b9"),
                        "map",
                        vec![Expr::Arrow(
                            vec!["m".to_owned()],
                            jsast::ArrowBody::Expr(Box::new(Expr::Object(vec![
                                ("hi".to_owned(), jsast::member(jsast::id("m"), "hi")),
                                ("lo".to_owned(), jsast::member(jsast::id("m"), "lo")),
                            ]))),
                        )],
                    ),
                ),
                Stmt::Return(Some(jsast::method_call(
                    jsast::id("bp"),
                    "map",
                    vec![Expr::Arrow(
                        vec!["m".to_owned()],
                        jsast::ArrowBody::Expr(Box::new(jsast::cond(
                            Expr::Binary(
                                BinOp::StrictNe,
                                Box::new(jsast::id("m")),
                                Box::new(Expr::Undefined),
                            ),
                            Expr::Object(vec![
                                ("hi".to_owned(), jsast::member(jsast::id("m"), "hi")),
                                ("lo".to_owned(), jsast::member(jsast::id("m"), "lo")),
                            ]),
                            Expr::Undefined,
                        ))),
                    )],
                ))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let s = jsast::stmt_to_string(&decl);
        assert!(
            !s.contains(".map("),
            "expected both .map() clones elided: {s}"
        );
        assert!(
            s.contains("if (63 < i)"),
            "expected simplified single guard 63 < i: {s}"
        );
        assert!(
            !s.contains("48 < i"),
            "expected dominated 48 < i dropped: {s}"
        );
    }

    #[test]
    fn hoists_as_mut_slice_and_versions_labeled_masked_probe_loop() {
        let mut decl = Stmt::FunctionDecl {
            name: "refine_like".to_owned(),
            params: vec!["aw".to_owned(), "av".to_owned(), "h0".to_owned()],
            body: vec![
                jsast::lets(vec![
                    ("r".to_owned(), None),
                    ("q".to_owned(), None),
                    ("G".to_owned(), None),
                ]),
                jsast::assign_stmt(
                    jsast::id("r"),
                    jsast::call(
                        jsast::id("alloc$vec$impl_1_as_mut_slice$h1234"),
                        vec![jsast::id("aw")],
                    ),
                ),
                jsast::assign_stmt(
                    jsast::id("q"),
                    Expr::Binary(
                        BinOp::UShr,
                        Box::new(Expr::Binary(
                            BinOp::BitAnd,
                            Box::new(jsast::id("h0")),
                            Box::new(jsast::id("av")),
                        )),
                        Box::new(Expr::Num(0.0)),
                    ),
                ),
                Stmt::Labeled(
                    "a".to_owned(),
                    Box::new(Stmt::Block(vec![Stmt::LoopForever(
                        None,
                        vec![
                            jsast::assign_stmt(
                                jsast::id("G"),
                                jsast::member(jsast::id("r"), "len"),
                            ),
                            Stmt::If(
                                Expr::Unary(
                                    UnOp::Not,
                                    Box::new(Expr::Binary(
                                        BinOp::Lt,
                                        Box::new(jsast::id("q")),
                                        Box::new(jsast::id("G")),
                                    )),
                                ),
                                vec![Stmt::ExprStmt(jsast::call(
                                    jsast::id("core$panicking$panic_bounds_check$h1"),
                                    vec![jsast::id("q"), jsast::id("G")],
                                ))],
                                None,
                            ),
                            Stmt::If(
                                Expr::Binary(
                                    BinOp::StrictEq,
                                    Box::new(jsast::index(
                                        jsast::member(jsast::id("r"), "buf"),
                                        Expr::Binary(
                                            BinOp::Add,
                                            Box::new(jsast::member(jsast::id("r"), "off")),
                                            Box::new(jsast::id("q")),
                                        ),
                                    )),
                                    Box::new(Expr::Num(0.0)),
                                ),
                                vec![Stmt::Break(Some("a".to_owned()))],
                                None,
                            ),
                            jsast::assign_stmt(
                                jsast::id("q"),
                                Expr::Binary(
                                    BinOp::UShr,
                                    Box::new(Expr::Binary(
                                        BinOp::BitAnd,
                                        Box::new(Expr::Binary(
                                            BinOp::Add,
                                            Box::new(jsast::id("q")),
                                            Box::new(Expr::Num(1.0)),
                                        )),
                                        Box::new(jsast::id("av")),
                                    )),
                                    Box::new(Expr::Num(0.0)),
                                ),
                            ),
                        ],
                    )])),
                ),
                Stmt::Return(Some(jsast::id("q"))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let s = jsast::stmt_to_string(&decl);
        assert!(s.contains("$hb_0 = r.buf;"), "expected r.buf hoisted: {s}");
        assert!(s.contains("$hl_0 = r.len;"), "expected r.len hoisted: {s}");
        assert!(
            s.contains("av >>> 0 < $hl_0"),
            "expected pre-loop masked guard: {s}"
        );
        assert_eq!(
            s.matches("panic_bounds_check").count(),
            1,
            "expected bounds check stripped from fast loop: {s}"
        );
    }

    #[test]
    fn straight_line_prop_does_not_leak_past_labeled_block_break() {
        // Reproduces 04_iter_range / 05_slice_index: a small unrolled loop triggers
        // `prop_straight_line_lits`, which must NOT propagate `res = "many"` past `a: { if (q) { res = "bookends"; break a; } res = "many"; }`.
        let mut decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: vec!["q".to_owned(), "a".to_owned()],
            body: vec![
                jsast::lets(vec![("m".to_owned(), None), ("res".to_owned(), None)]),
                jsast::assign_stmt(jsast::id("m"), Expr::Num(0.0)),
                Stmt::While(
                    Expr::Binary(
                        BinOp::Lt,
                        Box::new(jsast::id("m")),
                        Box::new(Expr::Num(3.0)),
                    ),
                    vec![
                        jsast::assign_stmt(
                            jsast::index(jsast::id("a"), jsast::id("m")),
                            jsast::id("m"),
                        ),
                        jsast::assign_stmt(
                            jsast::id("m"),
                            Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("m")),
                                Box::new(Expr::Num(1.0)),
                            ),
                        ),
                    ],
                ),
                Stmt::Labeled(
                    "lbl".to_owned(),
                    Box::new(Stmt::Block(vec![
                        Stmt::If(
                            jsast::id("q"),
                            vec![
                                jsast::assign_stmt(
                                    jsast::id("res"),
                                    Expr::Str("bookends".to_owned()),
                                ),
                                Stmt::Break(Some("lbl".to_owned())),
                            ],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("res"), Expr::Str("many".to_owned())),
                    ])),
                ),
                Stmt::Return(Some(jsast::id("res"))),
            ],
        };
        run_speed_post_opts(&mut decl);
        let s = jsast::stmt_to_string(&decl);
        assert!(
            s.contains("return res;"),
            "expected `res` preserved after labeled block break: {s}"
        );
        assert!(
            s.contains("\"bookends\""),
            "expected `bookends` branch preserved: {s}"
        );
    }
}
