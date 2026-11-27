//! Whole-program `BigInt` object property `(lo, hi)` splitting.

use std::collections::{BTreeMap, BTreeSet};

use crate::jsast::{
    self, walk_expr, walk_stmt, ArrowBody, BinOp, Expr, Stmt, UnOp, Visit, VisitMut,
};

use super::exclusions::{collect_js_text_tokens_and_strings, ProgramExclusions};
use super::LoweredRow;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnpackPart {
    Lo,
    Hi,
}

fn is_bigint_32(expr: &Expr) -> bool {
    match expr {
        Expr::BigInt(d) => d == "32",
        Expr::Call(c, args) => {
            matches!(&**c, Expr::Ident(n) if n == "BigInt")
                && args.len() == 1
                && (matches!(&args[0], Expr::Str(s) if s == "32")
                    || matches!(&args[0], Expr::Num(n) if *n == 32.0))
        }
        _ => false,
    }
}

fn extract_bigint_call_arg(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(c, args) = expr else {
        return None;
    };
    if matches!(&**c, Expr::Ident(n) if n == "BigInt") && args.len() == 1 {
        Some(&args[0])
    } else {
        None
    }
}

fn parse_bigint_const(expr: &Expr) -> Option<(i32, i32)> {
    fn parse_i128(e: &Expr) -> Option<i128> {
        match e {
            Expr::BigInt(s) => s.parse::<i128>().ok(),
            Expr::Unary(UnOp::Neg, inner) => parse_i128(inner)?.checked_neg(),
            Expr::Call(c, args)
                if matches!(&**c, Expr::Ident(n) if n == "BigInt") && args.len() == 1 =>
            {
                match &args[0] {
                    Expr::Str(s) => s.parse::<i128>().ok(),
                    Expr::Num(n) if n.is_finite() && n.fract() == 0.0 => {
                        if *n >= (i64::MIN as f64) && *n <= (u64::MAX as f64) {
                            Some(*n as i128)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
    let val = parse_i128(expr)?;
    if (i64::MIN as i128..=u64::MAX as i128).contains(&val) {
        Some((val as i32, (val >> 32) as i32))
    } else {
        None
    }
}

fn is_pure_no_call(expr: &Expr) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_) => true,
        Expr::Member(obj, _) => is_pure_no_call(obj),
        Expr::Index(obj, idx) => is_pure_no_call(obj) && is_pure_no_call(idx),
        Expr::Unary(_, inner) => is_pure_no_call(inner),
        Expr::Binary(_, l, r) => is_pure_no_call(l) && is_pure_no_call(r),
        Expr::Cond(t, c, a) => is_pure_no_call(t) && is_pure_no_call(c) && is_pure_no_call(a),
        _ => false,
    }
}

fn is_pure_obj(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(_) => true,
        Expr::Member(inner, _) => is_pure_obj(inner),
        Expr::Index(inner, idx) => {
            is_pure_obj(inner) && matches!(&**idx, Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_))
        }
        _ => false,
    }
}

fn is_unsigned_32_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Binary(BinOp::UShr, _, r) => matches!(&**r, Expr::Num(n) if *n == 0.0),
        Expr::Num(n) => n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n <= (u32::MAX as f64),
        Expr::RawNum(s) => s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .map_or_else(
                || s.parse::<u32>().is_ok(),
                |hex| u32::from_str_radix(hex, 16).is_ok(),
            ),
        _ => false,
    }
}

fn check_pack_pair<'a>(shifted: &'a Expr, low: &'a Expr) -> Option<(&'a Expr, &'a Expr)> {
    let Expr::Binary(BinOp::Shl, hi_call, shift) = shifted else {
        return None;
    };
    if !is_bigint_32(shift) {
        return None;
    }
    let hi_arg = extract_bigint_call_arg(hi_call)?;
    let lo_arg = extract_bigint_call_arg(low)?;
    if is_pure_no_call(lo_arg) && is_pure_no_call(hi_arg) && is_unsigned_32_expr(lo_arg) {
        Some((lo_arg, hi_arg))
    } else {
        None
    }
}

fn extract_pack_bigint(expr: &Expr) -> Option<(&Expr, &Expr)> {
    let Expr::Binary(BinOp::BitOr, lhs, rhs) = expr else {
        return None;
    };
    check_pack_pair(lhs, rhs).or_else(|| check_pack_pair(rhs, lhs))
}

enum PackedRhs<'a> {
    Const,
    Pack(&'a Expr, &'a Expr),
    MemberCopy(&'a Expr),
}

fn classify_prop_rhs<'a>(expr: &'a Expr, prop: &str) -> Option<PackedRhs<'a>> {
    if parse_bigint_const(expr).is_some() {
        return Some(PackedRhs::Const);
    }
    if let Some((lo, hi)) = extract_pack_bigint(expr) {
        return Some(PackedRhs::Pack(lo, hi));
    }
    if let Expr::Member(obj, k) = expr {
        if k == prop && is_pure_obj(obj) {
            return Some(PackedRhs::MemberCopy(obj));
        }
    }
    None
}

fn match_unpack_bigint_part(expr: &Expr) -> Option<(UnpackPart, &Expr)> {
    let inner = match expr {
        Expr::Binary(BinOp::BitOr, inner, z) if matches!(&**z, Expr::Num(n) if *n == 0.0) => {
            inner.as_ref()
        }
        _ => expr,
    };
    let Expr::Call(num_callee, num_args) = inner else {
        return None;
    };
    if !matches!(&**num_callee, Expr::Ident(n) if n == "Number") || num_args.len() != 1 {
        return None;
    }
    let Expr::Call(as_int_callee, as_int_args) = &num_args[0] else {
        return None;
    };
    let Expr::Member(bi_obj, method) = &**as_int_callee else {
        return None;
    };
    if !matches!(&**bi_obj, Expr::Ident(n) if n == "BigInt")
        || method != "asIntN"
        || as_int_args.len() != 2
        || !matches!(&as_int_args[0], Expr::Num(n) if *n == 32.0)
    {
        return None;
    }
    match &as_int_args[1] {
        Expr::Binary(BinOp::Shr, base, shift) if is_bigint_32(shift) => {
            Some((UnpackPart::Hi, base.as_ref()))
        }
        other => Some((UnpackPart::Lo, other)),
    }
}

fn to_clean_i32_expr(mut expr: Expr) -> Expr {
    if let Expr::Binary(BinOp::UShr, inner, z) = expr {
        if matches!(&*z, Expr::Num(n) if *n == 0.0) {
            expr = *inner;
        } else {
            expr = Expr::Binary(BinOp::UShr, inner, z);
        }
    }
    match &expr {
        Expr::Num(n)
            if n.is_finite()
                && n.fract() == 0.0
                && *n >= (i32::MIN as f64)
                && *n <= (i32::MAX as f64) =>
        {
            expr
        }
        Expr::Binary(BinOp::BitOr, _, z) if matches!(&**z, Expr::Num(n) if *n == 0.0) => expr,
        _ => jsast::i32_wrap(expr),
    }
}

fn expr_mentions_ident(expr: &Expr, target: &str) -> bool {
    struct V<'a> {
        target: &'a str,
        found: bool,
    }
    impl Visit for V<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if self.found {
                return;
            }
            let hit = match stmt {
                Stmt::Let(n, _) => n == self.target,
                Stmt::Lets(decls) => decls.iter().any(|(n, _)| n == self.target),
                Stmt::TryCatch { binding, .. } => binding == self.target,
                Stmt::FunctionDecl { name, params, .. } => {
                    name == self.target || params.iter().any(|p| p == self.target)
                }
                Stmt::Raw(s) => s.contains(self.target),
                _ => false,
            };
            if hit {
                self.found = true;
            } else {
                walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, expr: &Expr) {
            if self.found {
                return;
            }
            let hit = match expr {
                Expr::Ident(n) => n == self.target,
                Expr::Arrow(params, _) => params.iter().any(|p| p == self.target),
                Expr::Raw(s) => s.contains(self.target),
                _ => false,
            };
            if hit {
                self.found = true;
            } else {
                walk_expr(self, expr);
            }
        }
    }
    let mut v = V {
        target,
        found: false,
    };
    v.visit_expr(expr);
    v.found
}

fn collect_fn_unpack_temps(params: &[String], body: &[Stmt]) -> BTreeMap<String, String> {
    struct TempScan {
        decl_count: BTreeMap<String, usize>,
        assign_prop: BTreeMap<String, Option<String>>,
        read_ok: BTreeMap<String, bool>,
        bad_names: BTreeSet<String>,
        all_idents: BTreeSet<String>,
    }

    struct MarkBad<'a>(&'a mut TempScan);
    impl Visit for MarkBad<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Const(n, _) | Stmt::Let(n, _) => self.0.mark_bad(n),
                Stmt::Lets(decls) => {
                    for (n, _) in decls {
                        self.0.mark_bad(n);
                    }
                }
                Stmt::TryCatch { binding, .. } => self.0.mark_bad(binding),
                Stmt::FunctionDecl { name, params, .. } => {
                    self.0.mark_bad(name);
                    for p in params {
                        self.0.mark_bad(p);
                    }
                }
                Stmt::Raw(text) => {
                    let mut dummy = BTreeSet::new();
                    collect_js_text_tokens_and_strings(text, &mut self.0.bad_names, &mut dummy);
                }
                _ => {}
            }
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Ident(name) => self.0.mark_bad(name),
                Expr::Arrow(params, _) => {
                    for p in params {
                        self.0.mark_bad(p);
                    }
                }
                Expr::Raw(text) => {
                    let mut dummy = BTreeSet::new();
                    collect_js_text_tokens_and_strings(text, &mut self.0.bad_names, &mut dummy);
                }
                _ => {}
            }
            walk_expr(self, expr);
        }
    }

    impl TempScan {
        fn mark_bad(&mut self, name: &str) {
            self.bad_names.insert(name.to_owned());
            self.all_idents.insert(name.to_owned());
        }

        fn mark_idents_bad_in_stmts(&mut self, stmts: &[Stmt]) {
            MarkBad(self).visit_stmts(stmts);
        }

        fn mark_idents_bad_in_expr(&mut self, expr: &Expr) {
            MarkBad(self).visit_expr(expr);
        }

        fn record_assign(&mut self, name: &str, rhs: &Expr) {
            self.all_idents.insert(name.to_owned());
            if let Expr::Member(obj, prop) = rhs {
                if is_pure_obj(obj) && !expr_mentions_ident(obj, name) {
                    self.visit_expr(obj);
                    match self.assign_prop.get(name) {
                        None => {
                            self.assign_prop.insert(name.to_owned(), Some(prop.clone()));
                        }
                        Some(Some(prev)) if prev == prop => {}
                        _ => {
                            self.assign_prop.insert(name.to_owned(), None);
                            self.mark_bad(name);
                        }
                    }
                    return;
                }
            }
            self.assign_prop.insert(name.to_owned(), None);
            self.mark_bad(name);
            self.visit_expr(rhs);
        }
    }

    impl Visit for TempScan {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                    if matches!(lhs.as_ref(), Expr::Ident(_)) =>
                {
                    if let Expr::Ident(name) = lhs.as_ref() {
                        self.record_assign(name, rhs);
                    }
                }
                Stmt::Const(n, e) => {
                    self.mark_bad(n);
                    self.visit_expr(e);
                }
                Stmt::Let(n, init) => {
                    self.all_idents.insert(n.clone());
                    *self.decl_count.entry(n.clone()).or_insert(0) += 1;
                    if let Some(rhs) = init {
                        self.record_assign(n, rhs);
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, init) in decls {
                        self.all_idents.insert(n.clone());
                        *self.decl_count.entry(n.clone()).or_insert(0) += 1;
                        if let Some(rhs) = init {
                            self.record_assign(n, rhs);
                        }
                    }
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    self.mark_bad(binding);
                    self.visit_stmts(body);
                    self.visit_stmts(handler);
                }
                Stmt::FunctionDecl { name, params, body } => {
                    self.mark_bad(name);
                    for p in params {
                        self.mark_bad(p);
                    }
                    self.mark_idents_bad_in_stmts(body);
                }
                Stmt::Raw(text) => {
                    let mut dummy = BTreeSet::new();
                    collect_js_text_tokens_and_strings(text, &mut self.bad_names, &mut dummy);
                }
                _ => walk_stmt(self, stmt),
            }
        }

        fn visit_expr(&mut self, expr: &Expr) {
            if let Some((_, Expr::Ident(name))) = match_unpack_bigint_part(expr) {
                self.all_idents.insert(name.clone());
                self.read_ok.entry(name.clone()).or_insert(true);
                return;
            }
            match expr {
                Expr::Ident(name) => {
                    self.all_idents.insert(name.clone());
                    self.read_ok.insert(name.clone(), false);
                    self.mark_bad(name);
                }
                Expr::Arrow(params, body) => {
                    for p in params {
                        self.mark_bad(p);
                    }
                    match body {
                        ArrowBody::Expr(b) => self.mark_idents_bad_in_expr(b),
                        ArrowBody::Block(stmts) => self.mark_idents_bad_in_stmts(stmts),
                    }
                }
                Expr::Raw(text) => {
                    let mut dummy = BTreeSet::new();
                    collect_js_text_tokens_and_strings(text, &mut self.bad_names, &mut dummy);
                }
                _ => walk_expr(self, expr),
            }
        }
    }

    let mut scan = TempScan {
        decl_count: BTreeMap::new(),
        assign_prop: BTreeMap::new(),
        read_ok: BTreeMap::new(),
        bad_names: params.iter().cloned().collect(),
        all_idents: params.iter().cloned().collect(),
    };
    scan.visit_stmts(body);

    let mut result = BTreeMap::new();
    for (name, count) in &scan.decl_count {
        if *count != 1 || scan.bad_names.contains(name) {
            continue;
        }
        let Some(Some(prop)) = scan.assign_prop.get(name) else {
            continue;
        };
        if scan.read_ok.get(name) != Some(&true) {
            continue;
        }
        let lo_name = format!("{name}$lo");
        let hi_name = format!("{name}$hi");
        if scan.all_idents.contains(&lo_name)
            || scan.all_idents.contains(&hi_name)
            || scan.bad_names.contains(&lo_name)
            || scan.bad_names.contains(&hi_name)
        {
            continue;
        }
        result.insert(name.clone(), prop.clone());
    }
    result
}

fn is_fixed_blacklisted_prop(name: &str) -> bool {
    matches!(
        name,
        "_0" | "_1"
            | "_2"
            | "_3"
            | "_4"
            | "_5"
            | "_6"
            | "_7"
            | "_8"
            | "_9"
            | "TAG"
            | "buf"
            | "off"
            | "len"
            | "sc"
            | "ptr"
            | "meta"
            | rcgjs_object::ptr::BUF
            | rcgjs_object::ptr::OFF
            | rcgjs_object::ptr::LEN
            | rcgjs_object::ptr::SCALE
            | rcgjs_object::ptr::WINDOW
            | rcgjs_object::ptr::PTR
            | rcgjs_object::ptr::META
    ) || name.ends_with("$lo")
        || name.ends_with("$hi")
}

struct BigIntPropScanner {
    excluded: BTreeSet<String>,
    has_pack_write: BTreeSet<String>,
}

impl BigIntPropScanner {
    fn exclude_prop_and_base(&mut self, name: &str) {
        self.excluded.insert(name.to_owned());
        if let Some(base) = name
            .strip_suffix("$lo")
            .or_else(|| name.strip_suffix("$hi"))
        {
            self.excluded.insert(base.to_owned());
        }
    }

    fn scan_stmt(&mut self, stmt: &Stmt, temp_props: &BTreeMap<String, String>) {
        struct S<'a, 'b> {
            scanner: &'a mut BigIntPropScanner,
            temp_props: &'b BTreeMap<String, String>,
        }
        impl Visit for S<'_, '_> {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                match stmt {
                    Stmt::FunctionDecl { params, body, .. } => {
                        let inner_temps = collect_fn_unpack_temps(params, body);
                        S {
                            scanner: self.scanner,
                            temp_props: &inner_temps,
                        }
                        .visit_stmts(body);
                    }
                    Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => match (lhs.as_ref(), rhs.as_ref()) {
                        (Expr::Ident(v), Expr::Member(obj, k))
                            if self.temp_props.get(v).is_some_and(|p| p == k)
                                && is_pure_obj(obj) =>
                        {
                            self.visit_expr(obj);
                        }
                        (Expr::Member(lhs_obj, k), _) => {
                            self.visit_expr(lhs_obj);
                            if is_pure_obj(lhs_obj) {
                                match classify_prop_rhs(rhs, k) {
                                    Some(PackedRhs::Const) => {
                                        self.scanner.has_pack_write.insert(k.clone());
                                    }
                                    Some(PackedRhs::Pack(lo, hi)) => {
                                        self.scanner.has_pack_write.insert(k.clone());
                                        self.visit_expr(lo);
                                        self.visit_expr(hi);
                                    }
                                    Some(PackedRhs::MemberCopy(rhs_obj)) => {
                                        self.visit_expr(rhs_obj);
                                    }
                                    None => {
                                        self.scanner.exclude_prop_and_base(k);
                                        self.visit_expr(rhs);
                                    }
                                }
                            } else {
                                self.scanner.exclude_prop_and_base(k);
                                self.visit_expr(rhs);
                            }
                        }
                        _ => {
                            self.visit_expr(lhs);
                            self.visit_expr(rhs);
                        }
                    },
                    Stmt::Let(n, Some(Expr::Member(obj, k)))
                        if self.temp_props.get(n).is_some_and(|p| p == k) && is_pure_obj(obj) =>
                    {
                        self.visit_expr(obj);
                    }
                    Stmt::Lets(decls) => {
                        for (n, init) in decls {
                            if let Some(e) = init {
                                if let Expr::Member(obj, k) = e {
                                    if self.temp_props.get(n).is_some_and(|p| p == k)
                                        && is_pure_obj(obj)
                                    {
                                        self.visit_expr(obj);
                                        continue;
                                    }
                                }
                                self.visit_expr(e);
                            }
                        }
                    }
                    _ => walk_stmt(self, stmt),
                }
            }

            fn visit_expr(&mut self, expr: &Expr) {
                if let Some((_, inner)) = match_unpack_bigint_part(expr) {
                    match inner {
                        Expr::Member(obj, _) if is_pure_obj(obj) => {
                            self.visit_expr(obj);
                            return;
                        }
                        Expr::Ident(v) if self.temp_props.contains_key(v) => return,
                        _ => {}
                    }
                }
                match expr {
                    Expr::Str(s) => self.scanner.exclude_prop_and_base(s),
                    Expr::Object(entries) => {
                        let mut seen_keys = BTreeSet::new();
                        for (k, val) in entries {
                            if !seen_keys.insert(k.as_str()) {
                                self.scanner.exclude_prop_and_base(k);
                            }
                            if is_fixed_blacklisted_prop(k) || self.scanner.excluded.contains(k) {
                                self.scanner.exclude_prop_and_base(k);
                                self.visit_expr(val);
                                continue;
                            }
                            match classify_prop_rhs(val, k) {
                                Some(PackedRhs::Const) => {
                                    self.scanner.has_pack_write.insert(k.clone());
                                }
                                Some(PackedRhs::Pack(lo, hi)) => {
                                    self.scanner.has_pack_write.insert(k.clone());
                                    self.visit_expr(lo);
                                    self.visit_expr(hi);
                                }
                                Some(PackedRhs::MemberCopy(rhs_obj)) => self.visit_expr(rhs_obj),
                                None => {
                                    self.scanner.exclude_prop_and_base(k);
                                    self.visit_expr(val);
                                }
                            }
                        }
                    }
                    Expr::Member(obj, k) => {
                        self.scanner.exclude_prop_and_base(k);
                        self.visit_expr(obj);
                    }
                    Expr::Arrow(_, body) => {
                        let empty = BTreeMap::new();
                        let mut inner = S {
                            scanner: self.scanner,
                            temp_props: &empty,
                        };
                        match body {
                            ArrowBody::Expr(b) => inner.visit_expr(b),
                            ArrowBody::Block(stmts) => inner.visit_stmts(stmts),
                        }
                    }
                    _ => walk_expr(self, expr),
                }
            }
        }
        S {
            scanner: self,
            temp_props,
        }
        .visit_stmt(stmt);
    }
}

struct BigIntPropRewriter<'a> {
    split_props: &'a BTreeSet<String>,
    temp_props: BTreeMap<String, String>,
    changed: bool,
}

impl BigIntPropRewriter<'_> {
    fn with_temp_props<F: FnOnce(&mut Self)>(&mut self, next: BTreeMap<String, String>, f: F) {
        let prev = std::mem::replace(&mut self.temp_props, next);
        f(self);
        self.temp_props = prev;
    }

    fn split_rhs_expr(&mut self, expr: Expr, prop: &str) -> (Expr, Expr) {
        if let Some((lo, hi)) = parse_bigint_const(&expr) {
            return (jsast::num(f64::from(lo)), jsast::num(f64::from(hi)));
        }
        if let Some((lo_ref, hi_ref)) = extract_pack_bigint(&expr) {
            let mut lo = to_clean_i32_expr(lo_ref.clone());
            let mut hi = to_clean_i32_expr(hi_ref.clone());
            self.with_temp_props(BTreeMap::new(), |this| {
                this.visit_expr_mut(&mut lo);
                this.visit_expr_mut(&mut hi);
            });
            return (lo, hi);
        }
        if let Expr::Member(mut obj, k) = expr {
            debug_assert_eq!(k, prop);
            self.with_temp_props(BTreeMap::new(), |this| {
                this.visit_expr_mut(&mut obj);
            });
            return (
                Expr::Member(obj.clone(), format!("{prop}$lo")),
                Expr::Member(obj, format!("{prop}$hi")),
            );
        }
        unreachable!("validated split_rhs_expr must match Const, Pack, or MemberCopy");
    }

    /// Expands statement forms that split into multiple statements into `out`.
    /// Returns `None` if `stmt` was expanded and pushed into `out`, or `Some(stmt)` otherwise.
    fn try_expand_stmt(&mut self, stmt: Stmt, out: &mut Vec<Stmt>) -> Option<Stmt> {
        match stmt {
            Stmt::Let(n, init)
                if self
                    .temp_props
                    .get(&n)
                    .is_some_and(|p| self.split_props.contains(p)) =>
            {
                let p = self.temp_props[&n].clone();
                self.changed = true;
                match init {
                    None => out.push(Stmt::Lets(vec![
                        (format!("{n}$lo"), None),
                        (format!("{n}$hi"), None),
                    ])),
                    Some(Expr::Member(mut obj, _)) => {
                        self.visit_expr_mut(&mut obj);
                        out.push(Stmt::Let(
                            format!("{n}$lo"),
                            Some(Expr::Member(obj.clone(), format!("{p}$lo"))),
                        ));
                        out.push(Stmt::Let(
                            format!("{n}$hi"),
                            Some(Expr::Member(obj, format!("{p}$hi"))),
                        ));
                    }
                    Some(_) => unreachable!("validated temp init"),
                }
                None
            }
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => match (&*lhs, &*rhs) {
                (Expr::Ident(v), Expr::Member(obj, k))
                    if self.temp_props.get(v).is_some_and(|p| p == k)
                        && self.split_props.contains(k) =>
                {
                    let mut obj_cloned = (**obj).clone();
                    self.visit_expr_mut(&mut obj_cloned);
                    self.changed = true;
                    out.push(Stmt::ExprStmt(jsast::assign(
                        jsast::id(format!("{v}$lo")),
                        Expr::Member(Box::new(obj_cloned.clone()), format!("{k}$lo")),
                    )));
                    out.push(Stmt::ExprStmt(jsast::assign(
                        jsast::id(format!("{v}$hi")),
                        Expr::Member(Box::new(obj_cloned), format!("{k}$hi")),
                    )));
                    None
                }
                (Expr::Member(lhs_obj, k), _) if self.split_props.contains(k) => {
                    let mut lhs_obj = (**lhs_obj).clone();
                    self.visit_expr_mut(&mut lhs_obj);
                    let prop = k.clone();
                    let (lo_rhs, hi_rhs) = self.split_rhs_expr(*rhs, &prop);
                    self.changed = true;
                    out.push(Stmt::ExprStmt(jsast::assign(
                        Expr::Member(Box::new(lhs_obj.clone()), format!("{prop}$lo")),
                        lo_rhs,
                    )));
                    out.push(Stmt::ExprStmt(jsast::assign(
                        Expr::Member(Box::new(lhs_obj), format!("{prop}$hi")),
                        hi_rhs,
                    )));
                    None
                }
                _ => Some(Stmt::ExprStmt(Expr::Assign(lhs, rhs))),
            },
            other => Some(other),
        }
    }
}

impl VisitMut for BigIntPropRewriter<'_> {
    fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in std::mem::take(stmts) {
            if let Some(mut kept) = self.try_expand_stmt(stmt, &mut out) {
                self.visit_stmt_mut(&mut kept);
                out.push(kept);
            }
        }
        *stmts = out;
    }

    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::FunctionDecl { params, body, .. } => {
                let inner_temps = collect_fn_unpack_temps(params, body)
                    .into_iter()
                    .filter(|(_, p)| self.split_props.contains(p))
                    .collect();
                self.with_temp_props(inner_temps, |this| this.visit_block_mut(body));
            }
            Stmt::Lets(decls) => {
                let mut new_decls = Vec::with_capacity(decls.len());
                for (n, mut init) in std::mem::take(decls) {
                    if let Some(p) = self
                        .temp_props
                        .get(&n)
                        .filter(|p| self.split_props.contains(*p))
                        .cloned()
                    {
                        self.changed = true;
                        match init {
                            None => {
                                new_decls.push((format!("{n}$lo"), None));
                                new_decls.push((format!("{n}$hi"), None));
                            }
                            Some(Expr::Member(mut obj, _)) => {
                                self.visit_expr_mut(&mut obj);
                                new_decls.push((
                                    format!("{n}$lo"),
                                    Some(Expr::Member(obj.clone(), format!("{p}$lo"))),
                                ));
                                new_decls.push((
                                    format!("{n}$hi"),
                                    Some(Expr::Member(obj, format!("{p}$hi"))),
                                ));
                            }
                            Some(_) => unreachable!("validated temp lets init"),
                        }
                    } else {
                        if let Some(e) = &mut init {
                            self.visit_expr_mut(e);
                        }
                        new_decls.push((n, init));
                    }
                }
                *decls = new_decls;
            }
            _ => jsast::walk_stmt_mut(self, stmt),
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if let Some((part, inner)) = match_unpack_bigint_part(expr) {
            let suffix = match part {
                UnpackPart::Lo => "$lo",
                UnpackPart::Hi => "$hi",
            };
            match inner {
                Expr::Member(obj, k) if self.split_props.contains(k) => {
                    let mut new_obj = (**obj).clone();
                    self.visit_expr_mut(&mut new_obj);
                    *expr = Expr::Member(Box::new(new_obj), format!("{k}{suffix}"));
                    self.changed = true;
                    return;
                }
                Expr::Ident(v)
                    if self
                        .temp_props
                        .get(v)
                        .is_some_and(|p| self.split_props.contains(p)) =>
                {
                    *expr = Expr::Ident(format!("{v}{suffix}"));
                    self.changed = true;
                    return;
                }
                _ => {}
            }
        }
        match expr {
            Expr::Object(entries) => {
                let mut new_entries = Vec::with_capacity(entries.len());
                for (k, mut v) in std::mem::take(entries) {
                    if self.split_props.contains(&k) {
                        let (lo_expr, hi_expr) = self.split_rhs_expr(v, &k);
                        new_entries.push((format!("{k}$lo"), lo_expr));
                        new_entries.push((format!("{k}$hi"), hi_expr));
                        self.changed = true;
                    } else {
                        self.visit_expr_mut(&mut v);
                        new_entries.push((k, v));
                    }
                }
                *entries = new_entries;
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => {
                self.with_temp_props(BTreeMap::new(), |this| this.visit_expr_mut(b));
            }
            Expr::Arrow(_, ArrowBody::Block(stmts)) => {
                self.with_temp_props(BTreeMap::new(), |this| this.visit_block_mut(stmts));
            }
            _ => jsast::walk_expr_mut(self, expr),
        }
    }
}

pub(super) fn split_bigint_props(lowered: &mut [LoweredRow], exclusions: &ProgramExclusions) {
    let mut scanner = BigIntPropScanner {
        excluded: BTreeSet::new(),
        has_pack_write: BTreeSet::new(),
    };
    for s in exclusions
        .raw_tokens
        .iter()
        .chain(exclusions.raw_strings.iter())
        .chain(exclusions.boundary_fields.iter())
    {
        scanner.exclude_prop_and_base(s);
    }

    let empty = BTreeMap::new();
    for (_, res, _, _) in lowered.iter() {
        if let Ok(item) = res {
            scanner.scan_stmt(&item.decl, &empty);
        }
    }

    let split_props: BTreeSet<String> = scanner
        .has_pack_write
        .into_iter()
        .filter(|p| !is_fixed_blacklisted_prop(p) && !scanner.excluded.contains(p))
        .collect();
    if split_props.is_empty() {
        return;
    }

    for (item_name, res, _, _) in lowered.iter_mut() {
        let Ok(item) = res else { continue };
        let mut rewriter = BigIntPropRewriter {
            split_props: &split_props,
            temp_props: BTreeMap::new(),
            changed: false,
        };
        rewriter.visit_stmt_mut(&mut item.decl);
        if rewriter.changed {
            item.refs = super::super::lane::refs_of(&item.decl, item_name);
        }
    }
}
