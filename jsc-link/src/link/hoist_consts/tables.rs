//! Read-only scalar lookup table, nested scalar array, and constant slice hoisting,
//! plus TypedArray wrapping for read-only numeric tables under `ReleaseSpeed`.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{ItemKind, JsName, LinkItem};
use crate::jsast::{self, walk_expr, walk_stmt, Expr, Stmt, UnOp, Visit, VisitMut};

use super::exclusions::ProgramExclusions;
use super::LoweredRow;

#[derive(Clone, Copy)]
pub(super) enum ReadOnlyKind {
    ScalarTable { min_len: usize },
    NestedArray,
    ConstSlice,
}

impl ReadOnlyKind {
    /// Checks whether `rhs` is a valid constant initializer for this kind.
    /// Returns `Some(nested_elems)` on match (`true` for nested aggregate elements,
    /// `false` for flat scalar elements).
    fn match_init(self, rhs: &Expr, allow_bigint: bool) -> Option<bool> {
        match self {
            Self::ScalarTable { min_len } => {
                is_scalar_array_lit(rhs, min_len, allow_bigint).then_some(false)
            }
            Self::NestedArray => is_nested_scalar_array_lit(rhs, 8, allow_bigint).then_some(true),
            Self::ConstSlice => is_pure_const_slice_lit(rhs, allow_bigint),
        }
    }
}

pub(super) fn is_pure_scalar_lit(expr: &Expr, allow_bigint: bool) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
        Expr::BigInt(_) => allow_bigint,
        Expr::Unary(op, inner) => {
            matches!(op, UnOp::Neg | UnOp::Pos | UnOp::Not | UnOp::BitNot)
                && is_pure_scalar_lit(inner, allow_bigint)
        }
        _ => false,
    }
}

pub(super) fn is_scalar_array_lit(expr: &Expr, min_len: usize, allow_bigint: bool) -> bool {
    let Expr::Array(elems) = expr else {
        return false;
    };
    elems.len() >= min_len
        && elems
            .iter()
            .all(|elem| is_pure_scalar_lit(elem, allow_bigint))
}

fn is_flat_scalar_aggregate(expr: &Expr, allow_bigint: bool) -> bool {
    match expr {
        Expr::Array(elems) => {
            !elems.is_empty()
                && elems.len() <= 16
                && elems
                    .iter()
                    .all(|elem| is_pure_scalar_lit(elem, allow_bigint))
        }
        Expr::Object(entries) => {
            !entries.is_empty()
                && entries.len() <= 16
                && entries
                    .iter()
                    .all(|(_, v)| is_pure_scalar_lit(v, allow_bigint))
        }
        _ => false,
    }
}

fn is_nested_scalar_array_lit(expr: &Expr, min_len: usize, allow_bigint: bool) -> bool {
    let Expr::Array(elems) = expr else {
        return false;
    };
    elems.len() >= min_len
        && elems
            .iter()
            .all(|elem| is_flat_scalar_aggregate(elem, allow_bigint))
}

/// Matches a constant slice literal `{ buf: [...], off: 0, len: N }` (`N >= 4`) whose backing
/// array holds either pure scalars or flat pure-scalar tuples/objects (e.g. Keccak's `IOTA`).
fn is_pure_const_slice_lit(expr: &Expr, allow_bigint: bool) -> Option<bool> {
    let Expr::Object(entries) = expr else {
        return None;
    };
    if entries.len() != 3 {
        return None;
    }
    let ((k0, buf), (k1, off), (k2, len)) = (&entries[0], &entries[1], &entries[2]);
    if k0 != "buf" || k1 != "off" || k2 != "len" {
        return None;
    }
    if !matches!(off, Expr::Num(_) | Expr::RawNum(_))
        || !matches!(len, Expr::Num(_) | Expr::RawNum(_))
    {
        return None;
    }
    let Expr::Array(elems) = buf else {
        return None;
    };
    if elems.len() < 4 {
        return None;
    }
    if elems
        .iter()
        .all(|elem| is_pure_scalar_lit(elem, allow_bigint))
    {
        Some(false)
    } else if elems
        .iter()
        .all(|elem| is_flat_scalar_aggregate(elem, allow_bigint))
    {
        Some(true)
    } else {
        None
    }
}

/// Collects `let` bindings declared in `stmts` (recursing through nested control-flow statements
/// such as loops and conditionals, without entering nested functions or arrows).
pub(super) fn collect_declared_locals(
    stmts: &[Stmt],
    params: &BTreeSet<String>,
    out: &mut Vec<String>,
) {
    struct V<'a> {
        params: &'a BTreeSet<String>,
        out: &'a mut Vec<String>,
    }
    impl Visit for V<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Let(n, _) => {
                    if !self.params.contains(n) {
                        self.out.push(n.clone());
                    }
                }
                Stmt::Lets(decls) => {
                    for (n, _) in decls {
                        if !self.params.contains(n) {
                            self.out.push(n.clone());
                        }
                    }
                }
                Stmt::FunctionDecl { .. } => {}
                _ => walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &Expr) {}
    }
    V { params, out }.visit_stmts(stmts);
}

fn is_ptr_eq_callee(expr: &Expr) -> bool {
    match expr {
        Expr::Member(ns, method) => {
            matches!(ns.as_ref(), Expr::Ident(id) if id == "__rt") && method == "ptr_eq"
        }
        Expr::Ident(id) => id == "$rt_ptr_eq",
        _ => false,
    }
}

fn is_derived_slice_expr(expr: &Expr, slice_aliases: &BTreeSet<String>) -> bool {
    match expr {
        Expr::Ident(name) => slice_aliases.contains(name),
        Expr::Object(entries) if entries.len() == 2 || entries.len() == 3 => {
            let has_buf = entries.iter().any(|(k, v)| {
                k == "buf"
                    && matches!(
                        v,
                        Expr::Member(base, prop)
                            if prop == "buf"
                                && matches!(base.as_ref(), Expr::Ident(n) if slice_aliases.contains(n))
                    )
            });
            let keys_ok = entries
                .iter()
                .all(|(k, _)| matches!(k.as_str(), "buf" | "off" | "len"));
            has_buf && keys_ok
        }
        _ => false,
    }
}

/// If `expr` is a direct element read of the underlying buffer (`local[idx]` for
/// `ScalarTable`/`NestedArray`, or `<slice_alias>.buf[idx]` for `ConstSlice`), returns `idx`.
fn elem_read_index<'a>(
    kind: ReadOnlyKind,
    expr: &'a Expr,
    local: &str,
    slice_aliases: &BTreeSet<String>,
) -> Option<&'a Expr> {
    let Expr::Index(obj, idx) = expr else {
        return None;
    };
    let matches_base = match kind {
        ReadOnlyKind::ScalarTable { .. } | ReadOnlyKind::NestedArray => {
            matches!(obj.as_ref(), Expr::Ident(o) if o == local)
        }
        ReadOnlyKind::ConstSlice => matches!(
            obj.as_ref(),
            Expr::Member(base, prop)
                if prop == "buf"
                    && matches!(base.as_ref(), Expr::Ident(n) if slice_aliases.contains(n))
        ),
    };
    matches_base.then_some(idx.as_ref())
}

/// Pass A: Finds the single declaration and constant initializer of `local` for `kind`,
/// rejecting duplicate declarations, conflicting/non-literal assignments, shadowing, or raw JS.
struct LocalInitScan<'a> {
    local: &'a str,
    kind: ReadOnlyKind,
    allow_bigint: bool,
    decl_count: usize,
    found_init: Option<(Expr, bool)>,
    bad: bool,
}

impl LocalInitScan<'_> {
    fn check_init_rhs(&mut self, rhs: &Expr) {
        let Some(nested) = self.kind.match_init(rhs, self.allow_bigint) else {
            self.bad = true;
            return;
        };
        if let Some((prev, prev_nested)) = &self.found_init {
            if prev != rhs || *prev_nested != nested {
                self.bad = true;
            }
            return;
        }
        self.found_init = Some((rhs.clone(), nested));
    }

    fn record_decl(&mut self, init: &Option<Expr>) {
        self.decl_count += 1;
        if self.decl_count > 1 {
            self.bad = true;
            return;
        }
        if let Some(rhs) = init {
            if self.found_init.is_some() {
                self.bad = true;
                return;
            }
            self.check_init_rhs(rhs);
        }
    }
}

impl Visit for LocalInitScan<'_> {
    fn visit_stmt(&mut self, s: &Stmt) {
        if self.bad {
            return;
        }
        match s {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if n == self.local) =>
            {
                self.check_init_rhs(rhs);
            }
            Stmt::ExprStmt(Expr::Seq(items)) => {
                for item in items {
                    if let Expr::Assign(lhs, rhs) = item {
                        if matches!(lhs.as_ref(), Expr::Ident(n) if n == self.local) {
                            self.check_init_rhs(rhs);
                            continue;
                        }
                    }
                    self.visit_expr(item);
                }
            }
            Stmt::Let(n, init) if n == self.local => self.record_decl(init),
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if n == self.local {
                        self.record_decl(init);
                    } else if let Some(e) = init {
                        self.visit_expr(e);
                    }
                }
            }
            Stmt::Const(n, _) if n == self.local => self.bad = true,
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                if binding == self.local {
                    self.bad = true;
                    return;
                }
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { name, params, body } => {
                if name == self.local || params.iter().any(|p| p == self.local) {
                    self.bad = true;
                    return;
                }
                self.visit_stmts(body);
            }
            Stmt::Raw(_) => self.bad = true,
            _ => walk_stmt(self, s),
        }
    }

    fn visit_expr(&mut self, e: &Expr) {
        if self.bad {
            return;
        }
        if let Expr::Arrow(params, _) = e {
            if params.iter().any(|p| p == self.local) {
                self.bad = true;
                return;
            }
        }
        walk_expr(self, e);
    }
}

/// Pass B: Collects derived slice aliases (for `ConstSlice`) and element aliases
/// (for `NestedArray` and nested `ConstSlice`).
fn collect_read_only_aliases(
    body: &[Stmt],
    local: &str,
    kind: ReadOnlyKind,
    nested_elems: bool,
) -> (BTreeSet<String>, BTreeSet<String>) {
    if matches!(kind, ReadOnlyKind::ScalarTable { .. }) {
        return (BTreeSet::new(), BTreeSet::new());
    }

    struct AliasPass<'a> {
        kind: ReadOnlyKind,
        local: &'a str,
        slice_aliases: &'a mut BTreeSet<String>,
        elem_aliases: &'a mut BTreeSet<String>,
        nested_elems: bool,
        changed: bool,
    }
    impl AliasPass<'_> {
        fn record(&mut self, dst: &str, rhs: &Expr) {
            let added = match self.kind {
                ReadOnlyKind::NestedArray => {
                    dst != self.local
                        && elem_read_index(self.kind, rhs, self.local, self.slice_aliases).is_some()
                        && self.elem_aliases.insert(dst.to_owned())
                }
                ReadOnlyKind::ConstSlice => {
                    (is_derived_slice_expr(rhs, self.slice_aliases)
                        && self.slice_aliases.insert(dst.to_owned()))
                        || (self.nested_elems
                            && elem_read_index(self.kind, rhs, self.local, self.slice_aliases)
                                .is_some()
                            && self.elem_aliases.insert(dst.to_owned()))
                }
                ReadOnlyKind::ScalarTable { .. } => false,
            };
            self.changed |= added;
        }
    }
    impl Visit for AliasPass<'_> {
        fn visit_stmt(&mut self, s: &Stmt) {
            match s {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                    if let Expr::Ident(dst) = lhs.as_ref() {
                        self.record(dst, rhs);
                    }
                }
                Stmt::ExprStmt(Expr::Seq(items)) => {
                    for item in items {
                        if let Expr::Assign(lhs, rhs) = item {
                            if let Expr::Ident(dst) = lhs.as_ref() {
                                self.record(dst, rhs);
                            }
                        }
                    }
                }
                Stmt::Let(dst, Some(rhs)) => self.record(dst, rhs),
                Stmt::Lets(decls) => {
                    for (dst, init) in decls {
                        if let Some(rhs) = init {
                            self.record(dst, rhs);
                        }
                    }
                }
                Stmt::FunctionDecl { .. } => {}
                _ => walk_stmt(self, s),
            }
        }
        fn visit_expr(&mut self, _: &Expr) {}
    }

    let mut slice_aliases = match kind {
        ReadOnlyKind::ConstSlice => BTreeSet::from([local.to_owned()]),
        _ => BTreeSet::new(),
    };
    let mut elem_aliases = BTreeSet::new();
    loop {
        let mut pass = AliasPass {
            kind,
            local,
            slice_aliases: &mut slice_aliases,
            elem_aliases: &mut elem_aliases,
            nested_elems,
            changed: false,
        };
        pass.visit_stmts(body);
        if !pass.changed || matches!(kind, ReadOnlyKind::NestedArray) {
            break;
        }
    }
    (slice_aliases, elem_aliases)
}

/// Pass C: Verifies that every use of `local` and its aliases across `body` is strictly read-only.
struct ReadOnlyUseVerifier<'a> {
    kind: ReadOnlyKind,
    local: &'a str,
    slice_aliases: &'a BTreeSet<String>,
    elem_aliases: &'a BTreeSet<String>,
    nested_elems: bool,
    reads: usize,
    bad: bool,
}

impl ReadOnlyUseVerifier<'_> {
    fn is_tracked(&self, name: &str) -> bool {
        name == self.local || self.slice_aliases.contains(name) || self.elem_aliases.contains(name)
    }

    fn check_derived_slice(&mut self, expr: &Expr) {
        if let Expr::Object(entries) = expr {
            for (k, v) in entries {
                if k == "buf" {
                    self.reads += 1;
                } else {
                    self.expr(v, false);
                }
            }
        } else {
            self.reads += 1;
        }
    }

    /// Handles binding/assignment to `local`, a slice alias, or an element alias.
    /// Returns `true` if `dst` was one of them.
    fn check_alias_write(&mut self, dst: &str, rhs: &Expr) -> bool {
        if dst == self.local {
            return true;
        }
        if matches!(self.kind, ReadOnlyKind::ConstSlice) && self.slice_aliases.contains(dst) {
            if is_derived_slice_expr(rhs, self.slice_aliases) {
                self.check_derived_slice(rhs);
            } else {
                self.bad = true;
            }
            return true;
        }
        if self.elem_aliases.contains(dst) {
            if let Some(idx) = elem_read_index(self.kind, rhs, self.local, self.slice_aliases) {
                self.reads += 1;
                self.expr(idx, false);
            } else {
                self.bad = true;
            }
            return true;
        }
        false
    }

    fn check_assign(&mut self, lhs: &Expr, rhs: &Expr) {
        if let Expr::Ident(dst) = lhs {
            if self.check_alias_write(dst, rhs) {
                return;
            }
        }
        self.expr(lhs, true);
        self.expr(rhs, false);
    }

    fn expr(&mut self, e: &Expr, is_lhs: bool) {
        if self.bad {
            return;
        }
        match e {
            Expr::Ident(name) => {
                if self.is_tracked(name) {
                    self.bad = true;
                }
            }
            Expr::Index(obj, idx) => {
                if let Some(elem_idx) =
                    elem_read_index(self.kind, e, self.local, self.slice_aliases)
                {
                    if is_lhs || self.nested_elems {
                        self.bad = true;
                        return;
                    }
                    self.reads += 1;
                    self.expr(elem_idx, false);
                    return;
                }
                if matches!(obj.as_ref(), Expr::Ident(n) if self.elem_aliases.contains(n)) {
                    if is_lhs {
                        self.bad = true;
                        return;
                    }
                    self.reads += 1;
                    self.expr(idx, false);
                    return;
                }
                if !matches!(self.kind, ReadOnlyKind::ScalarTable { .. }) {
                    if let Some(outer_idx) =
                        elem_read_index(self.kind, obj, self.local, self.slice_aliases)
                    {
                        if is_lhs {
                            self.bad = true;
                            return;
                        }
                        self.reads += 1;
                        self.expr(outer_idx, false);
                        self.expr(idx, false);
                        return;
                    }
                }
                self.expr(obj, false);
                self.expr(idx, false);
            }
            Expr::Member(obj, prop) => {
                if let Expr::Ident(name) = obj.as_ref() {
                    if matches!(self.kind, ReadOnlyKind::ScalarTable { .. })
                        && !is_lhs
                        && prop == "length"
                        && name == self.local
                    {
                        self.reads += 1;
                        return;
                    }
                    if matches!(self.kind, ReadOnlyKind::ConstSlice)
                        && self.slice_aliases.contains(name)
                    {
                        if !is_lhs && matches!(prop.as_str(), "off" | "len") {
                            self.reads += 1;
                            return;
                        }
                        self.bad = true;
                        return;
                    }
                    if self.elem_aliases.contains(name) {
                        if !is_lhs {
                            self.reads += 1;
                            return;
                        }
                        self.bad = true;
                        return;
                    }
                }
                if matches!(self.kind, ReadOnlyKind::NestedArray) {
                    if let Some(inner_idx) =
                        elem_read_index(self.kind, obj, self.local, self.slice_aliases)
                    {
                        if is_lhs {
                            self.bad = true;
                            return;
                        }
                        self.reads += 1;
                        self.expr(inner_idx, false);
                        return;
                    }
                }
                self.expr(obj, false);
            }
            Expr::Call(callee, args)
                if matches!(self.kind, ReadOnlyKind::ConstSlice)
                    && is_ptr_eq_callee(callee)
                    && args.len() == 2 =>
            {
                for a in args {
                    if matches!(a, Expr::Ident(n) if self.slice_aliases.contains(n)) {
                        self.reads += 1;
                    } else {
                        self.expr(a, false);
                    }
                }
            }
            Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                self.expr(l, true);
                self.expr(r, false);
            }
            Expr::Arrow(params, _) => {
                if params.iter().any(|p| self.is_tracked(p)) {
                    self.bad = true;
                    return;
                }
                walk_expr(self, e);
            }
            Expr::Raw(_) => self.bad = true,
            _ => walk_expr(self, e),
        }
    }
}

impl Visit for ReadOnlyUseVerifier<'_> {
    fn visit_stmt(&mut self, s: &Stmt) {
        if self.bad {
            return;
        }
        match s {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => self.check_assign(lhs, rhs),
            Stmt::ExprStmt(Expr::Seq(items)) => {
                for item in items {
                    if let Expr::Assign(lhs, rhs) = item {
                        self.check_assign(lhs, rhs);
                    } else {
                        self.expr(item, false);
                    }
                    if self.bad {
                        return;
                    }
                }
            }
            Stmt::Let(n, Some(rhs)) => {
                if !self.check_alias_write(n, rhs) {
                    self.expr(rhs, false);
                }
            }
            Stmt::Let(_, None) => {}
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(rhs) = init {
                        if !self.check_alias_write(n, rhs) {
                            self.expr(rhs, false);
                        }
                        if self.bad {
                            return;
                        }
                    }
                }
            }
            Stmt::Const(n, e) => {
                if self.is_tracked(n) {
                    self.bad = true;
                    return;
                }
                self.expr(e, false);
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                if self.is_tracked(binding) {
                    self.bad = true;
                    return;
                }
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { name, params, body } => {
                if self.is_tracked(name) || params.iter().any(|p| self.is_tracked(p)) {
                    self.bad = true;
                    return;
                }
                self.visit_stmts(body);
            }
            Stmt::Raw(_) => self.bad = true,
            _ => walk_stmt(self, s),
        }
    }

    fn visit_expr(&mut self, e: &Expr) {
        self.expr(e, false);
    }
}

/// Checks whether `local` across `body` is declared once, initialized once to a constant literal
/// matching `kind`, and only ever used in strictly read-only positions (never mutated, shadowed,
/// or escaped).
pub(super) fn find_read_only_local_init(
    body: &[Stmt],
    local: &str,
    allow_bigint: bool,
    kind: ReadOnlyKind,
) -> Option<Expr> {
    let mut scan = LocalInitScan {
        local,
        kind,
        allow_bigint,
        decl_count: 0,
        found_init: None,
        bad: false,
    };
    scan.visit_stmts(body);
    if scan.bad || scan.decl_count != 1 {
        return None;
    }
    let (init_expr, nested_elems) = scan.found_init?;

    let (slice_aliases, elem_aliases) = collect_read_only_aliases(body, local, kind, nested_elems);

    let mut verify = ReadOnlyUseVerifier {
        kind,
        local,
        slice_aliases: &slice_aliases,
        elem_aliases: &elem_aliases,
        nested_elems,
        reads: 0,
        bad: false,
    };
    verify.visit_stmts(body);
    (!verify.bad && verify.reads >= 1).then_some(init_expr)
}

pub(super) fn strip_hoisted_table_stmts(
    stmts: &mut Vec<Stmt>,
    hoisted: &BTreeSet<String>,
    min_len: usize,
) {
    struct Stripper<'a> {
        hoisted: &'a BTreeSet<String>,
        min_len: usize,
    }
    impl Stripper<'_> {
        fn is_hoisted_init_assign(&self, expr: &Expr) -> bool {
            matches!(
                expr,
                Expr::Assign(lhs, rhs)
                    if matches!(lhs.as_ref(), Expr::Ident(n) if self.hoisted.contains(n))
                        && (is_scalar_array_lit(rhs, self.min_len, true)
                            || is_nested_scalar_array_lit(rhs, 8, true)
                            || is_pure_const_slice_lit(rhs, true).is_some())
            )
        }

        fn retain_stmt(&self, stmt: &mut Stmt) -> bool {
            match stmt {
                Stmt::ExprStmt(e) if self.is_hoisted_init_assign(e) => false,
                Stmt::ExprStmt(Expr::Seq(items)) => {
                    items.retain(|item| !self.is_hoisted_init_assign(item));
                    if items.is_empty() {
                        return false;
                    }
                    if items.len() == 1 {
                        if let Some(single) = items.pop() {
                            *stmt = Stmt::ExprStmt(single);
                        }
                    }
                    true
                }
                Stmt::Let(n, _) if self.hoisted.contains(n) => false,
                Stmt::Lets(decls) => {
                    decls.retain(|(n, _)| !self.hoisted.contains(n));
                    !decls.is_empty()
                }
                _ => true,
            }
        }
    }
    impl VisitMut for Stripper<'_> {
        fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
            stmts.retain_mut(|stmt| self.retain_stmt(stmt));
            jsast::walk_block_mut(self, stmts);
        }

        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if let Stmt::Labeled(_, inner) = stmt {
                if !self.retain_stmt(inner) {
                    **inner = Stmt::Block(Vec::new());
                    return;
                }
            }
            jsast::walk_stmt_mut(self, stmt);
        }
    }

    Stripper { hoisted, min_len }.visit_block_mut(stmts);
}

pub(super) fn replace_ident_in_stmts(stmts: &mut [Stmt], from: &str, to: &str) {
    crate::lower::js::ast_util::rename_ident_stmts(stmts, from, to);
}

pub(super) fn exact_int_lit(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Num(n)
            if n.is_finite()
                && n.fract() == 0.0
                && !(*n == 0.0 && n.is_sign_negative())
                && *n >= f64::from(i32::MIN)
                && *n <= f64::from(u32::MAX) =>
        {
            Some(*n as i64)
        }
        Expr::Unary(UnOp::Neg, inner) => {
            let v = exact_int_lit(inner)?;
            if v == 0 {
                return None;
            }
            let neg = v.checked_neg()?;
            (neg >= i64::from(i32::MIN) && neg <= i64::from(u32::MAX)).then_some(neg)
        }
        Expr::RawNum(s) => s
            .parse::<i64>()
            .ok()
            .filter(|&v| v >= i64::from(i32::MIN) && v <= i64::from(u32::MAX)),
        _ => None,
    }
}

fn exact_f64_lit(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Num(n) if n.is_finite() => Some(*n),
        Expr::Unary(UnOp::Neg, inner) => exact_f64_lit(inner).map(|v| -v),
        Expr::Unary(UnOp::Pos, inner) => exact_f64_lit(inner),
        Expr::RawNum(s) => s.parse::<f64>().ok().filter(|v| v.is_finite()),
        _ => None,
    }
}

fn typed_num_array_ctor(elems: &[Expr], min_len: usize) -> Option<&'static str> {
    if elems.len() < min_len {
        return None;
    }
    let mut min_v = i64::MAX;
    let mut max_v = i64::MIN;
    let mut all_int = true;
    for e in elems {
        if let Some(v) = exact_int_lit(e) {
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        } else {
            all_int = false;
            break;
        }
    }
    if all_int {
        if min_v >= 0 && max_v <= 255 {
            return Some("Uint8Array");
        } else if min_v >= 0 && max_v <= 65535 {
            return Some("Uint16Array");
        } else if min_v >= i64::from(i32::MIN) && max_v <= i64::from(i32::MAX) {
            return Some("Int32Array");
        } else if min_v >= 0 && max_v <= i64::from(u32::MAX) {
            return Some("Uint32Array");
        }
    }
    if elems.iter().all(|e| exact_f64_lit(e).is_some()) {
        Some("Float64Array")
    } else {
        None
    }
}

pub(super) fn wrap_typed_num_array(expr: Expr, min_len: usize) -> Expr {
    if let Expr::Array(ref elems) = expr {
        if let Some(ctor) = typed_num_array_ctor(elems, min_len) {
            return Expr::New(Box::new(Expr::Ident(ctor.to_owned())), vec![expr]);
        }
    }
    expr
}

/// Upgrades internal read-only top-level integer static array items (`let T = [int, ...]` with
/// `len >= 4` whose only uses across all live items are read-only `T[idx]` / `T.length`) to
/// `const T = new <TypedArray>([...])` under `ReleaseSpeed`.
pub(super) fn upgrade_readonly_static_int_arrays(
    table: &BTreeMap<JsName, LinkItem>,
    lowered: &mut [LoweredRow],
    live: &BTreeSet<JsName>,
    exclusions: &ProgramExclusions,
) {
    struct UseChecker<'a> {
        cands: &'a mut BTreeSet<String>,
    }
    impl UseChecker<'_> {
        fn expr(&mut self, e: &Expr, is_lhs: bool) {
            if self.cands.is_empty() {
                return;
            }
            match e {
                Expr::Ident(id) => {
                    self.cands.remove(id);
                }
                Expr::Index(obj, idx) => {
                    if !is_lhs {
                        if let Expr::Ident(id) = obj.as_ref() {
                            if self.cands.contains(id) {
                                self.expr(idx, false);
                                return;
                            }
                        }
                    }
                    self.expr(obj, false);
                    self.expr(idx, false);
                }
                Expr::Member(obj, prop) => {
                    if !is_lhs && prop == "length" {
                        if let Expr::Ident(id) = obj.as_ref() {
                            if self.cands.contains(id) {
                                return;
                            }
                        }
                    }
                    self.expr(obj, false);
                }
                Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                    self.expr(l, true);
                    self.expr(r, false);
                }
                Expr::Arrow(params, _) => {
                    for p in params {
                        self.cands.remove(p);
                    }
                    walk_expr(self, e);
                }
                _ => walk_expr(self, e),
            }
        }
    }
    impl Visit for UseChecker<'_> {
        fn visit_stmt(&mut self, s: &Stmt) {
            if self.cands.is_empty() {
                return;
            }
            match s {
                Stmt::Const(n, _) | Stmt::Let(n, _) => {
                    self.cands.remove(n);
                }
                Stmt::Lets(decls) => {
                    for (n, _) in decls {
                        self.cands.remove(n);
                    }
                }
                Stmt::TryCatch { binding, .. } => {
                    self.cands.remove(binding);
                }
                Stmt::FunctionDecl { name, params, .. } => {
                    self.cands.remove(name);
                    for p in params {
                        self.cands.remove(p);
                    }
                }
                _ => {}
            }
            walk_stmt(self, s);
        }
        fn visit_expr(&mut self, e: &Expr) {
            self.expr(e, false);
        }
    }

    let lowered_ok: BTreeSet<&JsName> = lowered
        .iter()
        .filter_map(|(name, res, _, _)| res.is_ok().then_some(name))
        .collect();

    let mut cands: BTreeSet<String> = BTreeSet::new();
    for (item_name, res, _, _) in lowered.iter() {
        let Ok(item) = res else { continue };
        if !live.contains(item_name) {
            continue;
        }
        if table
            .get(item_name)
            .is_some_and(|t| t.linkage.exported || t.kind == ItemKind::Import)
        {
            continue;
        }
        let arr_opt = match &item.decl {
            Stmt::Let(n, Some(Expr::Array(elems))) | Stmt::Const(n, Expr::Array(elems))
                if n == item_name.as_str() =>
            {
                Some(elems)
            }
            _ => None,
        };
        let Some(elems) = arr_opt else { continue };
        if typed_num_array_ctor(elems, 4).is_some()
            && !exclusions.raw_tokens.contains(item_name.as_str())
        {
            cands.insert(item_name.as_str().to_owned());
        }
    }
    if cands.is_empty() {
        return;
    }

    // Disqualify any candidate referenced by a non-lowered live item (e.g. baseline fallback JS).
    for item in table.values() {
        if !live.contains(&item.name) || lowered_ok.contains(&item.name) {
            continue;
        }
        cands.retain(|c| !item.refs.iter().any(|r| r.as_str() == c) && !item.js.contains(c));
    }
    if cands.is_empty() {
        return;
    }

    // Verify every use across all lowered items is strictly read-only `T[idx]` or `T.length`.
    for (item_name, res, _, _) in lowered.iter() {
        if cands.is_empty() {
            return;
        }
        let Ok(item) = res else { continue };
        if cands.contains(item_name.as_str()) {
            // Skip scanning the candidate's own top-level declaration.
            continue;
        }
        let mut checker = UseChecker { cands: &mut cands };
        checker.visit_stmt(&item.decl);
    }
    if cands.is_empty() {
        return;
    }

    for (item_name, res, _, _) in lowered.iter_mut() {
        if !cands.contains(item_name.as_str()) {
            continue;
        }
        let Ok(item) = res else { continue };
        let decl = std::mem::replace(&mut item.decl, Stmt::Comment(String::new()));
        let upgraded = match decl {
            Stmt::Let(n, Some(arr)) | Stmt::Const(n, arr) => {
                Stmt::Const(n, wrap_typed_num_array(arr, 4))
            }
            other => other,
        };
        item.decl = upgraded;
        item.refs = super::super::lane::refs_of(&item.decl, item_name);
    }
}
