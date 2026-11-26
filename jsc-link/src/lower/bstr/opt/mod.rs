//! Post-lowering AST rewrite and optimization passes for `js-str=bytes`.

mod elide_ascii_in;
mod elide_ascii_out;
mod elide_bytes;
mod elide_sub;
mod fold_slice;

use std::collections::BTreeSet;

use crate::jsast::{self, walk_expr_mut, walk_stmt, walk_stmt_mut, Expr, Stmt, Visit, VisitMut};

use super::super::js::ast_util::rt_member;

#[cfg(test)]
pub(super) use elide_ascii_in::elide_ascii_only_bstr_in;
#[cfg(test)]
pub(super) use elide_ascii_out::elide_known_ascii_bstr_out;
#[cfg(test)]
pub(super) use elide_bytes::elide_readonly_bstr_bytes;
#[cfg(test)]
pub(super) use elide_sub::elide_proven_bstr_sub;

/// The byte-string twin of a hybrid helper, or `None` when `member` is not one.
fn twin(member: &str) -> Option<&'static str> {
    Some(match member {
        "str_bytes" => "bstr_bytes",
        "str_bytes_mut" => "bstr_bytes_mut",
        "bytes_str" => "bstr_of",
        "native_str_is_char_boundary" => "bstr_boundary",
        "native_str_utf16_index" | "native_str_byte_index" => "bstr_index",
        "str_to_lowercase" => "bstr_to_lowercase",
        "str_to_uppercase" => "bstr_to_uppercase",
        _ => return None,
    })
}

/// The hybrid helpers no byte-string program may call once [`rewrite`] has run.
const HYBRID_HELPERS: &[&str] = &[
    "str_bytes",
    "str_bytes_mut",
    "bytes_str",
    "str_len",
    "native_str_wf",
    "native_str_is_char_boundary",
    "native_str_utf16_index",
    "native_str_byte_index",
    "str_to_lowercase",
    "str_to_uppercase",
];

/// The first hybrid helper `js` still names, if any.
pub fn leftover_helper(js: &str) -> Option<&'static str> {
    HYBRID_HELPERS.iter().copied().find(|helper| {
        let needle = format!("__rt.{helper}");
        js.match_indices(&needle).any(|(at, _)| {
            let next = js[at + needle.len()..].chars().next();
            !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        })
    })
}

/// Rewrites one installed declaration from the hybrid helpers to the byte-string ones.
pub fn rewrite(stmt: &mut Stmt) {
    Rewriter.visit_stmt_mut(stmt);
}

struct Rewriter;

impl VisitMut for Rewriter {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if let Stmt::FunctionDecl { body, .. } = stmt {
            self.visit_stmts_mut(body);
            if crate::opts::get().profile == Some(crate::opts::Profile::ReleaseSpeed) {
                elide_bytes::elide_readonly_bstr_bytes(body);
                elide_ascii_in::elide_ascii_only_bstr_in(body);
                let known_ascii = elide_ascii_out::elide_known_ascii_bstr_out(body);
                elide_sub::elide_proven_bstr_sub(body, &known_ascii);
            }
            return;
        }
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        // Children first, so a helper's argument is already a byte-string expression.
        walk_expr_mut(self, expr);
        if rewrite_hybrid_rt_call(expr) {
            return;
        }
        if let Some(folded) = fold_slice::try_fold_bstr_slice_roundtrip(expr) {
            *expr = folded;
            return;
        }
        if unwrap_inline_ascii_bstr_in(expr) {
            return;
        }
        unwrap_inline_emptiness_bstr_in(expr);
    }
}

fn rewrite_hybrid_rt_call(expr: &mut Expr) -> bool {
    let Expr::Call(callee, args) = expr else {
        return false;
    };
    let Some(member) = rt_member(callee) else {
        return false;
    };
    match (member, args.len()) {
        ("str_len", 1) => {
            let string = args.pop().expect("one argument");
            *expr = match string {
                Expr::Str(text) => jsast::num(text.chars().count() as f64),
                string => jsast::member(string, "length"),
            };
            true
        }
        ("native_str_wf", 1) => {
            *expr = args.pop().expect("one argument");
            true
        }
        (member, _) => {
            if let Some(twin) = twin(member) {
                **callee = jsast::member(jsast::id("__rt"), twin);
            }
            false
        }
    }
}

fn unwrap_inline_ascii_bstr_in(expr: &mut Expr) -> bool {
    if let Expr::Binary(jsast::BinOp::StrictEq | jsast::BinOp::StrictNe, l, r) = expr {
        if is_ascii_str_lit(r) {
            if let Some(arg) = unwrap_bstr_in_call(l) {
                **l = arg;
                return true;
            }
        }
        if is_ascii_str_lit(l) {
            if let Some(arg) = unwrap_bstr_in_call(r) {
                **r = arg;
                return true;
            }
        }
    }
    // Inline ASCII-invariant method calls on `__rt.bstr_in(x)`:
    // `__rt.bstr_in(x).{startsWith,endsWith,includes}("ASCII")` -> `x.{...}("ASCII")`.
    if let Expr::Call(callee, args) = expr {
        if args.len() == 1 && is_ascii_str_lit(&args[0]) {
            if let Expr::Member(obj, method) = callee.as_mut() {
                if matches!(method.as_str(), "startsWith" | "endsWith" | "includes") {
                    if let Some(arg) = unwrap_bstr_in_call(obj) {
                        **obj = arg;
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Inline emptiness check: `__rt.bstr_in(x).length === 0` / `!== 0` / `> 0`.
fn unwrap_inline_emptiness_bstr_in(expr: &mut Expr) {
    let Expr::Binary(op, l, r) = expr else {
        return;
    };
    let zero_rhs = matches!(r.as_ref(), Expr::Num(n) if *n == 0.0);
    let zero_lhs = matches!(l.as_ref(), Expr::Num(n) if *n == 0.0);
    let target = if zero_rhs
        && matches!(
            op,
            jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Gt
        ) {
        Some(l.as_mut())
    } else if zero_lhs
        && matches!(
            op,
            jsast::BinOp::StrictEq | jsast::BinOp::StrictNe | jsast::BinOp::Lt
        )
    {
        Some(r.as_mut())
    } else {
        None
    };
    if let Some(Expr::Member(obj, prop)) = target {
        if prop == "length" {
            if let Some(arg) = unwrap_bstr_in_call(obj) {
                **obj = arg;
            }
        }
    }
}

fn is_ascii_str_lit(expr: &Expr) -> bool {
    matches!(expr, Expr::Str(s) if s.bytes().all(|b| b < 128))
}

fn unwrap_bstr_in_call(expr: &Expr) -> Option<Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if args.len() == 1 && rt_member(callee) == Some("bstr_in") {
        Some(args[0].clone())
    } else {
        None
    }
}

fn unwrap_bstr_bytes_call(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    if args.len() == 1 && rt_member(callee) == Some("bstr_bytes") {
        Some(&args[0])
    } else {
        None
    }
}

fn collect_fn_locals(stmts: &[Stmt], out: &mut BTreeSet<String>) {
    struct LocalCollector<'a>(&'a mut BTreeSet<String>);
    impl Visit for LocalCollector<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Let(n, _) | Stmt::Const(n, _) => {
                    self.0.insert(n.clone());
                }
                Stmt::Lets(decls) => {
                    for (n, _) in decls {
                        self.0.insert(n.clone());
                    }
                }
                Stmt::FunctionDecl { .. } => {}
                _ => walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &Expr) {}
    }
    LocalCollector(out).visit_stmts(stmts);
}

/// Collects local variables referenced inside nested closures (`Arrow` / inner `FunctionDecl`).
fn collect_closure_captured(stmts: &[Stmt], locals: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    crate::lower::js::ast_util::collect_closure_captured_btree(stmts, locals, out);
}
