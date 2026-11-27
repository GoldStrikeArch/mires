//! Zero-offset index simplification and repeated slice-load CSE passes:
//! - [`fold_zero_offset_arith`]: propagates dominated single-assignment `+0` locals and strips
//!   `0 + idx` / `idx + 0` inside array indexes and `{ buf, off [, len] }` literals.
//! - [`cse_slice_loads`]: hoists repeated `arr[off + idx]` / `arr[idx]` object-element receivers
//!   across contiguous read-only statement spans under `ReleaseSpeed`.

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, ArrowBody, Expr, Stmt, Visit,
    VisitMut,
};

use super::super::ast_util;

// ---------------------------------------------------------------------------
// fold_zero_offset_arith
// ---------------------------------------------------------------------------

fn is_pos_zero(expr: &Expr) -> bool {
    match expr {
        Expr::Num(n) => *n == 0.0 && !n.is_sign_negative(),
        Expr::RawNum(s) => s == "0",
        _ => false,
    }
}

#[derive(Default)]
struct ZeroWriteInfo {
    writes: usize,
    zero_writes: usize,
    disqualified: bool,
}

struct ZeroWriteScan<'a> {
    locals: &'a HashSet<String>,
    in_closure: bool,
    info: HashMap<String, ZeroWriteInfo>,
}

impl ZeroWriteScan<'_> {
    fn record_write(&mut self, name: &str, rhs: &Expr) {
        if self.locals.contains(name) {
            let e = self.info.entry(name.to_owned()).or_default();
            e.writes += 1;
            if self.in_closure {
                e.disqualified = true;
            } else if is_pos_zero(rhs) {
                e.zero_writes += 1;
            }
        }
    }
}

impl Visit for ZeroWriteScan<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Const(name, e) => {
                if self.locals.contains(name) {
                    self.info.entry(name.clone()).or_default().disqualified = true;
                }
                self.visit_expr(e);
            }
            Stmt::Let(name, Some(e)) => {
                self.record_write(name, e);
                self.visit_expr(e);
            }
            Stmt::Lets(decls) => {
                for (name, init) in decls {
                    if let Some(e) = init {
                        self.record_write(name, e);
                        self.visit_expr(e);
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
        match expr {
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(name) = lhs.as_ref() {
                    self.record_write(name, rhs);
                } else {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Ident(name) = lhs.as_ref() {
                    if self.locals.contains(name) {
                        let e = self.info.entry(name.clone()).or_default();
                        e.writes += 1;
                        e.disqualified = true;
                    }
                } else {
                    self.visit_expr(lhs);
                }
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

fn is_numeric_offset_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Num(n) => n.is_finite() && !(*n == 0.0 && n.is_sign_negative()),
        Expr::RawNum(_) | Expr::Ident(_) | Expr::Member(_, _) | Expr::Index(_, _) => true,
        Expr::Unary(jsast::UnOp::BitNot, inner) => is_numeric_offset_expr(inner),
        Expr::Unary(jsast::UnOp::Neg, inner) => {
            !is_pos_zero(inner) && is_numeric_offset_expr(inner)
        }
        Expr::Binary(
            jsast::BinOp::Add
            | jsast::BinOp::Sub
            | jsast::BinOp::Mul
            | jsast::BinOp::Div
            | jsast::BinOp::Rem
            | jsast::BinOp::BitAnd
            | jsast::BinOp::BitOr
            | jsast::BinOp::BitXor
            | jsast::BinOp::Shl
            | jsast::BinOp::Shr
            | jsast::BinOp::UShr,
            l,
            r,
        ) => is_numeric_offset_expr(l) && is_numeric_offset_expr(r),
        Expr::Cond(_, t, e) => is_numeric_offset_expr(t) && is_numeric_offset_expr(e),
        Expr::Call(callee, args) => matches!(
            callee.as_ref(),
            Expr::Member(obj, prop)
                if matches!(obj.as_ref(), Expr::Ident(id) if id == "Math")
                    && ((prop == "imul" && args.len() == 2)
                        || (prop == "clz32" && args.len() == 1))
        ),
        _ => false,
    }
}

fn simplify_offset(expr: &mut Expr) {
    if let Expr::Binary(jsast::BinOp::Add, l, r) = expr {
        if is_pos_zero(l) && is_numeric_offset_expr(r) {
            *expr = (**r).clone();
        } else if is_pos_zero(r) && is_numeric_offset_expr(l) {
            *expr = (**l).clone();
        }
    }
}

struct ZeroOffsetExprFolder<'a>(&'a HashSet<String>);

impl VisitMut for ZeroOffsetExprFolder<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Ident(name) => {
                if self.0.contains(name) {
                    *expr = jsast::num(0.0);
                }
            }
            Expr::Index(arr, idx) => {
                self.visit_expr_mut(arr);
                self.visit_expr_mut(idx);
                simplify_offset(idx);
            }
            Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                if !matches!(lhs.as_ref(), Expr::Ident(_)) {
                    self.visit_expr_mut(lhs);
                }
                self.visit_expr_mut(rhs);
            }
            Expr::Object(entries) => {
                for (_, val) in entries.iter_mut() {
                    self.visit_expr_mut(val);
                }
                let is_slot_or_slice = (entries.len() == 2
                    && entries[0].0 == rcgjs_object::ptr::BUF
                    && entries[1].0 == rcgjs_object::ptr::OFF)
                    || (entries.len() == 3
                        && entries[0].0 == rcgjs_object::ptr::BUF
                        && entries[1].0 == rcgjs_object::ptr::OFF
                        && entries[2].0 == rcgjs_object::ptr::LEN);
                if is_slot_or_slice {
                    simplify_offset(&mut entries[1].1);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => fold_zero_expr(b, &HashSet::new()),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => {
                let mut empty = HashSet::new();
                fold_zero_stmts(stmts, &HashSet::new(), &mut empty);
            }
            _ => walk_expr_mut(self, expr),
        }
    }
}

fn fold_zero_expr(expr: &mut Expr, active_zeros: &HashSet<String>) {
    ZeroOffsetExprFolder(active_zeros).visit_expr_mut(expr);
}

fn fold_zero_seq_stmt_expr(
    expr: &mut Expr,
    zero_candidates: &HashSet<String>,
    active_zeros: &mut HashSet<String>,
) {
    if let Expr::Seq(items) = expr {
        for item in items.iter_mut() {
            fold_zero_seq_stmt_expr(item, zero_candidates, active_zeros);
        }
        return;
    }
    fold_zero_expr(expr, active_zeros);
    if let Expr::Assign(lhs, rhs) = expr {
        if let Expr::Ident(name) = lhs.as_ref() {
            if zero_candidates.contains(name) && is_pos_zero(rhs) {
                active_zeros.insert(name.clone());
            }
        }
    }
}

fn fold_zero_stmts(
    stmts: &mut [Stmt],
    zero_candidates: &HashSet<String>,
    active_zeros: &mut HashSet<String>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) => fold_zero_seq_stmt_expr(e, zero_candidates, active_zeros),
            Stmt::Throw(e) | Stmt::Const(_, e) | Stmt::Return(Some(e)) => {
                fold_zero_expr(e, active_zeros);
            }
            Stmt::Let(name, Some(e)) => {
                fold_zero_expr(e, active_zeros);
                if zero_candidates.contains(name) && is_pos_zero(e) {
                    active_zeros.insert(name.clone());
                }
            }
            Stmt::Lets(decls) => {
                for (name, init) in decls {
                    if let Some(e) = init {
                        fold_zero_expr(e, active_zeros);
                        if zero_candidates.contains(name) && is_pos_zero(e) {
                            active_zeros.insert(name.clone());
                        }
                    }
                }
            }
            Stmt::If(test, then_b, else_b) => {
                fold_zero_expr(test, active_zeros);
                let mut then_z = active_zeros.clone();
                fold_zero_stmts(then_b, zero_candidates, &mut then_z);
                if let Some(els) = else_b {
                    let mut else_z = active_zeros.clone();
                    fold_zero_stmts(els, zero_candidates, &mut else_z);
                }
            }
            Stmt::Switch(disc, cases) => {
                fold_zero_expr(disc, active_zeros);
                for case in cases {
                    for t in &mut case.tests {
                        fold_zero_expr(t, active_zeros);
                    }
                    let mut case_z = active_zeros.clone();
                    fold_zero_stmts(&mut case.body, zero_candidates, &mut case_z);
                }
            }
            Stmt::Labeled(_, inner) => {
                let mut inner_z = active_zeros.clone();
                fold_zero_stmts(
                    std::slice::from_mut(inner.as_mut()),
                    zero_candidates,
                    &mut inner_z,
                );
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                let mut sub_z = active_zeros.clone();
                fold_zero_stmts(b, zero_candidates, &mut sub_z);
            }
            Stmt::FunctionDecl { body: b, .. } => {
                let mut empty = HashSet::new();
                fold_zero_stmts(b, &HashSet::new(), &mut empty);
            }
            Stmt::While(test, b) => {
                fold_zero_expr(test, active_zeros);
                let mut sub_z = active_zeros.clone();
                fold_zero_stmts(b, zero_candidates, &mut sub_z);
            }
            Stmt::TryCatch { body, handler, .. } => {
                let mut body_z = active_zeros.clone();
                fold_zero_stmts(body, zero_candidates, &mut body_z);
                let mut handler_z = active_zeros.clone();
                fold_zero_stmts(handler, zero_candidates, &mut handler_z);
            }
            _ => {}
        }
    }
}

fn collect_zero_candidates(body: &[Stmt], uninit_locals: &HashSet<String>) -> HashSet<String> {
    if uninit_locals.is_empty() {
        return HashSet::new();
    }
    let mut scan = ZeroWriteScan {
        locals: uninit_locals,
        in_closure: false,
        info: HashMap::new(),
    };
    scan.visit_stmts(body);
    scan.info
        .into_iter()
        .filter(|(_, w)| w.writes == 1 && w.zero_writes == 1 && !w.disqualified)
        .map(|(name, _)| name)
        .collect()
}

/// Simplifies zero-offset index arithmetic (`arr[0 + idx]` / `arr[idx + 0]` -> `arr[idx]`) and
/// slot/slice literals (`{ buf: arr, off: 0 + idx [, len: n] }`) when `idx` is a proven non-string
/// numeric offset expression, and propagates dominated single-assignment `+0` local variables
/// (such as `v$buf = vec.buf, v$off = 0, v$len = vec.len; ... v$buf[v$off + i]`) to literal `0`.
///
/// Running this before `fold_offset_slot_derefs` and `cse_slice_loads` exposes `{ buf: arr, off: i }`
/// slot derefs and `arr[i].prop` repeated slice loads for zero-offset specialized slice parameters
/// (`off === 0`) and intra-procedural `Vec::as_slice` / `Vec::as_mut_slice` views.
pub(in super::super) fn fold_zero_offset_arith(body: &mut [Stmt], uninit_locals: &HashSet<String>) {
    let zero_candidates = collect_zero_candidates(body, uninit_locals);
    let mut active_zeros = HashSet::new();
    fold_zero_stmts(body, &zero_candidates, &mut active_zeros);
}

// ---------------------------------------------------------------------------
// cse_slice_loads
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SliceLoadKey {
    arr: String,
    off: Option<String>,
    idx: String,
}

impl SliceLoadKey {
    fn to_expr(&self) -> Expr {
        let index = match &self.off {
            Some(off) => Expr::Binary(
                jsast::BinOp::Add,
                Box::new(jsast::id(off.clone())),
                Box::new(jsast::id(self.idx.clone())),
            ),
            None => jsast::id(self.idx.clone()),
        };
        Expr::Index(Box::new(jsast::id(self.arr.clone())), Box::new(index))
    }

    fn mentions(&self, name: &str) -> bool {
        self.arr == name || self.idx == name || self.off.as_deref() == Some(name)
    }
}

fn match_slice_load(expr: &Expr) -> Option<SliceLoadKey> {
    let Expr::Index(arr, index) = expr else {
        return None;
    };
    let Expr::Ident(arr_name) = arr.as_ref() else {
        return None;
    };
    match index.as_ref() {
        Expr::Ident(idx_name) => Some(SliceLoadKey {
            arr: arr_name.clone(),
            off: None,
            idx: idx_name.clone(),
        }),
        Expr::Binary(jsast::BinOp::Add, l, r) => match (l.as_ref(), r.as_ref()) {
            (Expr::Ident(off_name), Expr::Ident(idx_name)) => Some(SliceLoadKey {
                arr: arr_name.clone(),
                off: Some(off_name.clone()),
                idx: idx_name.clone(),
            }),
            _ => None,
        },
        _ => None,
    }
}

fn is_non_mutating_call(callee: &Expr) -> bool {
    match callee {
        Expr::Member(obj, method) if matches!(obj.as_ref(), Expr::Ident(m) if m == "Math") => {
            matches!(
                method.as_str(),
                "abs"
                    | "imul"
                    | "min"
                    | "max"
                    | "sqrt"
                    | "floor"
                    | "ceil"
                    | "trunc"
                    | "fround"
                    | "round"
                    | "clz32"
            )
        }
        _ if matches!(
            ast_util::rt_member(callee),
            Some("bstr_bytes" | "str_bytes")
        ) =>
        {
            true
        }
        Expr::Ident(name) => {
            name.contains("panic_bounds_check")
                || name.contains("panic_const_")
                || name.contains("slice_index_")
                || name.contains("panic_explicit")
                || name.contains("rust_begin_unwind")
        }
        _ => false,
    }
}

/// Collects `SliceLoadKey`s unconditionally evaluated as `arr[off + idx].prop` inside `expr`
/// without crossing short-circuit operators, conditional branches, or impure calls.
fn collect_unconditional_in_expr(expr: &Expr, out: &mut Vec<SliceLoadKey>) {
    match expr {
        Expr::Member(obj, _) => {
            if let Some(key) = match_slice_load(obj) {
                if !out.contains(&key) {
                    out.push(key);
                }
            } else {
                collect_unconditional_in_expr(obj, out);
            }
        }
        Expr::Unary(_, inner) => collect_unconditional_in_expr(inner, out),
        Expr::Binary(op, l, r) => {
            collect_unconditional_in_expr(l, out);
            if !matches!(op, jsast::BinOp::And | jsast::BinOp::Or) && !ast_util::expr_has_call(l) {
                collect_unconditional_in_expr(r, out);
            }
        }
        Expr::Cond(c, _, _) => collect_unconditional_in_expr(c, out),
        Expr::Index(o, i) => {
            collect_unconditional_in_expr(o, out);
            if !ast_util::expr_has_call(o) {
                collect_unconditional_in_expr(i, out);
            }
        }
        Expr::Assign(lhs, rhs) => {
            if let Expr::Member(obj, _) = lhs.as_ref() {
                if let Some(key) = match_slice_load(obj) {
                    if !out.contains(&key) {
                        out.push(key);
                    }
                }
            }
            collect_unconditional_in_expr(rhs, out);
        }
        _ => {}
    }
}

fn collect_unconditional_in_stmt(stmt: &Stmt, out: &mut Vec<SliceLoadKey>) {
    match stmt {
        Stmt::ExprStmt(e) | Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => {
            collect_unconditional_in_expr(e, out);
        }
        Stmt::If(test, _, _) | Stmt::Switch(test, _) => {
            collect_unconditional_in_expr(test, out);
        }
        _ => {}
    }
}

struct InvalidationChecker<'a> {
    key: &'a SliceLoadKey,
    found: bool,
}

impl Visit for InvalidationChecker<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if self.found {
            return;
        }
        match stmt {
            Stmt::Let(n, _) | Stmt::Const(n, _) if self.key.mentions(n) => {
                self.found = true;
                return;
            }
            Stmt::Lets(decls) if decls.iter().any(|(n, _)| self.key.mentions(n)) => {
                self.found = true;
                return;
            }
            Stmt::TryCatch { .. } | Stmt::FunctionDecl { .. } => {
                self.found = true;
                return;
            }
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
                if matches!(lhs.as_ref(), Expr::Index(_, _))
                    || matches!(lhs.as_ref(), Expr::Ident(name) if self.key.mentions(name)) =>
            {
                self.found = true;
                return;
            }
            Expr::Call(callee, _) if !is_non_mutating_call(callee) => {
                self.found = true;
                return;
            }
            Expr::New(_, _) | Expr::Arrow(_, _) => {
                self.found = true;
                return;
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

fn stmt_invalidates(stmt: &Stmt, key: &SliceLoadKey) -> bool {
    let mut checker = InvalidationChecker { key, found: false };
    checker.visit_stmt(stmt);
    checker.found
}

struct SliceLoadCounter<'a> {
    key: &'a SliceLoadKey,
    count: usize,
}

impl Visit for SliceLoadCounter<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if matches!(stmt, Stmt::TryCatch { .. } | Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Member(obj, _) if match_slice_load(obj).as_ref() == Some(self.key) => {
                self.count += 1;
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr(self, expr),
        }
    }
}

fn count_slice_loads_in_stmt(stmt: &Stmt, key: &SliceLoadKey) -> usize {
    let mut counter = SliceLoadCounter { key, count: 0 };
    counter.visit_stmt(stmt);
    counter.count
}

struct SliceLoadReplacer<'a> {
    key: &'a SliceLoadKey,
    var: &'a str,
}

impl VisitMut for SliceLoadReplacer<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::TryCatch { .. } | Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Member(obj, _) if match_slice_load(obj).as_ref() == Some(self.key) => {
                **obj = jsast::id(self.var.to_owned());
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr_mut(self, expr),
        }
    }
}

fn fresh_slice_temp(
    counter: &mut usize,
    declarations: &mut Vec<(String, Option<Expr>)>,
    local_names: &mut HashSet<String>,
) -> String {
    loop {
        let candidate = format!("$sl{}", *counter);
        *counter += 1;
        if local_names.insert(candidate.clone()) {
            declarations.push((candidate.clone(), None));
            return candidate;
        }
    }
}

fn cse_slice_loads_in_block(
    stmts: &mut Vec<Stmt>,
    counter: &mut usize,
    declarations: &mut Vec<(String, Option<Expr>)>,
    local_names: &mut HashSet<String>,
) {
    let mut i = 0;
    while i < stmts.len() {
        let mut candidates = Vec::new();
        collect_unconditional_in_stmt(&stmts[i], &mut candidates);
        let mut hoisted_here = Vec::new();
        for key in candidates {
            // Find the maximal contiguous span `[i..=last_use]` with no invalidating statement.
            let mut total_count = 0usize;
            let mut last_use = i;
            let mut valid_start = false;
            for (j, s) in stmts.iter().enumerate().skip(i) {
                if stmt_invalidates(s, &key) {
                    break;
                }
                if j == i {
                    valid_start = true;
                }
                let c = count_slice_loads_in_stmt(s, &key);
                if c > 0 {
                    total_count += c;
                    last_use = j;
                }
            }
            if !valid_start || total_count < 2 {
                continue;
            }
            let var = fresh_slice_temp(counter, declarations, local_names);
            let mut replacer = SliceLoadReplacer {
                key: &key,
                var: &var,
            };
            for s in &mut stmts[i..=last_use] {
                replacer.visit_stmt_mut(s);
            }
            hoisted_here.push(jsast::assign_stmt(jsast::id(var), key.to_expr()));
        }
        if !hoisted_here.is_empty() {
            let inserted = hoisted_here.len();
            for (offset, bind) in hoisted_here.into_iter().enumerate() {
                stmts.insert(i + offset, bind);
            }
            i += inserted + 1;
        } else {
            i += 1;
        }
    }
}

/// Common-subexpression elimination for repeated slice/array object-element reads such as
/// `a[b + h].z`, `a[b + h].x`, `a[b + h].y`, `a[b + h].prev`, `a[b + h].next` (`ReleaseSpeed`,
/// proxy-free arrays).
///
/// Whenever `arr[off + idx]` (or `arr[idx]`) is unconditionally evaluated as the receiver of a
/// member access in statement `stmts[start]` and appears at least twice across a maximal
/// contiguous read-only span `stmts[start..=end]` (no writes to `arr`/`off`/`idx`, no array
/// element writes `_[_] = ...`, no non-pure/non-panic calls, no closures or `try/catch`), we bind
/// `$slN = arr[off + idx]` right before `stmts[start]` and replace every `arr[off + idx].prop`
/// base across `stmts[start..=end]` with `$slN`.
pub(in super::super) fn cse_slice_loads(
    body: &mut Vec<Stmt>,
    declarations: &mut Vec<(String, Option<Expr>)>,
    local_names: &mut HashSet<String>,
) {
    let mut counter = 0usize;
    ast_util::visit_local_blocks_mut(body, true, |stmts| {
        cse_slice_loads_in_block(stmts, &mut counter, declarations, local_names);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cse_slice_loads_hoists_across_panic_check_and_invalidates_on_mutation() {
        let elem = || {
            Expr::Index(
                Box::new(jsast::id("a")),
                Box::new(Expr::Binary(
                    jsast::BinOp::Add,
                    Box::new(jsast::id("b")),
                    Box::new(jsast::id("i")),
                )),
            )
        };
        let panic_check = Stmt::If(
            Expr::Unary(
                jsast::UnOp::Not,
                Box::new(Expr::Binary(
                    jsast::BinOp::Lt,
                    Box::new(jsast::id("i")),
                    Box::new(jsast::id("len")),
                )),
            ),
            vec![Stmt::ExprStmt(Expr::Call(
                Box::new(jsast::id("core$panicking$panic_bounds_check$h1")),
                vec![
                    jsast::id("i"),
                    jsast::id("len"),
                    Expr::Object(vec![(
                        "filename".to_owned(),
                        jsast::rt_call("bstr_bytes", vec![jsast::string("a.rs")]),
                    )]),
                ],
            ))],
            None,
        );
        let mut body = vec![
            jsast::assign_stmt(
                jsast::id("x"),
                Expr::Member(Box::new(elem()), "x".to_owned()),
            ),
            panic_check,
            jsast::assign_stmt(
                jsast::id("y"),
                Expr::Member(Box::new(elem()), "y".to_owned()),
            ),
            // Mutating `a` must invalidate the cached load before reading `.z`.
            jsast::assign_stmt(
                Expr::Index(Box::new(jsast::id("a")), Box::new(jsast::num(0.0))),
                jsast::id("other"),
            ),
            jsast::assign_stmt(
                jsast::id("z"),
                Expr::Member(Box::new(elem()), "z".to_owned()),
            ),
        ];
        let mut declarations: Vec<(String, Option<Expr>)> = vec![
            ("x".to_owned(), None),
            ("y".to_owned(), None),
            ("z".to_owned(), None),
        ];
        let mut local_names: HashSet<String> = ["x".to_owned(), "y".to_owned(), "z".to_owned()]
            .into_iter()
            .collect();
        cse_slice_loads(&mut body, &mut declarations, &mut local_names);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert_eq!(
            printed.matches("a[b + i]").count(),
            2,
            "first two reads must share one hoisted temp, third read after store must reload:\n{printed}"
        );
        assert!(
            declarations.iter().any(|(l, _)| l.starts_with("$sl")),
            "expected hoisted slice temp in {declarations:?}"
        );
    }

    #[test]
    fn fold_zero_offset_arith_simplifies_indexes_and_slot_literals() {
        let mut body = vec![
            // Single-assignment zero offset local inside a slice unpack sequence:
            // `buf = v.buf, off = 0, len = v.len; loop { w = buf[off + i]; }`
            Stmt::ExprStmt(Expr::Seq(vec![
                Expr::Assign(
                    Box::new(jsast::id("buf")),
                    Box::new(jsast::member(jsast::id("v"), "buf")),
                ),
                Expr::Assign(Box::new(jsast::id("off")), Box::new(jsast::num(0.0))),
                Expr::Assign(
                    Box::new(jsast::id("len")),
                    Box::new(jsast::member(jsast::id("v"), "len")),
                ),
            ])),
            Stmt::LoopForever(
                None,
                vec![jsast::assign_stmt(
                    jsast::id("w"),
                    jsast::index(
                        jsast::id("buf"),
                        jsast::binary(jsast::BinOp::Add, jsast::id("off"), jsast::id("i")),
                    ),
                )],
            ),
            // Reassigned local `mut_off` MUST NOT fold.
            jsast::assign_stmt(jsast::id("mut_off"), jsast::num(0.0)),
            jsast::assign_stmt(jsast::id("mut_off"), jsast::num(2.0)),
            jsast::assign_stmt(
                jsast::id("m"),
                jsast::index(
                    jsast::id("buf"),
                    jsast::binary(jsast::BinOp::Add, jsast::id("mut_off"), jsast::id("i")),
                ),
            ),
            // Branch-only `branch_off = 0` MUST NOT leak out of the branch.
            Stmt::If(
                jsast::id("cond"),
                vec![jsast::assign_stmt(jsast::id("branch_off"), jsast::num(0.0))],
                None,
            ),
            jsast::assign_stmt(
                jsast::id("b"),
                jsast::index(
                    jsast::id("buf"),
                    jsast::binary(jsast::BinOp::Add, jsast::id("branch_off"), jsast::id("i")),
                ),
            ),
            // `x = a[0 + i];` -> `x = a[i];`
            jsast::assign_stmt(
                jsast::id("x"),
                jsast::index(
                    jsast::id("a"),
                    jsast::binary(jsast::BinOp::Add, jsast::num(0.0), jsast::id("i")),
                ),
            ),
            // `y = a[0 + idx[k]];` -> `y = a[idx[k]];`
            jsast::assign_stmt(
                jsast::id("y"),
                jsast::index(
                    jsast::id("a"),
                    jsast::binary(
                        jsast::BinOp::Add,
                        jsast::num(0.0),
                        jsast::index(jsast::id("idx"), jsast::id("k")),
                    ),
                ),
            ),
            // `p = { $b: a, $o: 0 + i };` -> `p = { $b: a, $o: i };`
            jsast::assign_stmt(
                jsast::id("p"),
                jsast::object_of(vec![
                    ("$b", jsast::id("a")),
                    (
                        "$o",
                        jsast::binary(jsast::BinOp::Add, jsast::num(0.0), jsast::id("i")),
                    ),
                ]),
            ),
            // `z = 0 + s;` MUST NOT fold (could be string concatenation outside index/slot).
            jsast::assign_stmt(
                jsast::id("z"),
                jsast::binary(jsast::BinOp::Add, jsast::num(0.0), jsast::id("s")),
            ),
        ];
        let uninit: HashSet<String> = [
            "buf",
            "off",
            "len",
            "mut_off",
            "branch_off",
            "w",
            "m",
            "b",
            "x",
            "y",
            "p",
            "z",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        fold_zero_offset_arith(&mut body, &uninit);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(printed.contains("w = buf[i];"), "{printed}");
        assert!(printed.contains("m = buf[mut_off + i];"), "{printed}");
        assert!(printed.contains("b = buf[branch_off + i];"), "{printed}");
        assert!(printed.contains("x = a[i];"), "{printed}");
        assert!(printed.contains("y = a[idx[k]];"), "{printed}");
        assert!(printed.contains("p = { $b: a, $o: i };"), "{printed}");
        assert!(printed.contains("z = 0 + s;"), "{printed}");
    }
}
