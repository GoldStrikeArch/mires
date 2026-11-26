//! Intraprocedural JS-AST `KnownAscii` pass over a function body: proves local scalar strings and
//! local native `{ s, cap }` `String` objects contain only ASCII bytes (`0..128`), and elides
//! `__rt.bstr_out(e)` / `__rt.bstr_in(e)` / `__rt._bstr_is_ascii(e)` /
//! `__rt.bstr_to_{lower,upper}case(e)`.

use std::collections::{BTreeMap, BTreeSet};

use crate::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, ArrowBody, Expr, Stmt, Visit,
    VisitMut,
};

use super::super::super::js::ast_util::rt_member;
use super::{collect_closure_captured, collect_fn_locals};

#[derive(Clone, Copy)]
enum TrackKind {
    Scalar,
    Obj,
}

const TRACK_KINDS: [TrackKind; 2] = [TrackKind::Scalar, TrackKind::Obj];

#[derive(Default)]
struct AsciiTrackInfo {
    /// Plain writes (`x = rhs`, `o = { s: init, cap: ... }`, or `o.s = rhs`) that can ground.
    plain_writes: Vec<Expr>,
    /// Compound append writes (`x += rhs` or `o.s += rhs`) that cannot ground alone.
    append_writes: Vec<Expr>,
    bad: bool,
}

impl AsciiTrackInfo {
    fn all_writes_ascii(&self, known: &AsciiSets) -> bool {
        self.plain_writes
            .iter()
            .all(|e| is_known_ascii_expr(e, known, 0))
            && self
                .append_writes
                .iter()
                .all(|e| is_known_ascii_expr(e, known, 0) || is_numeric_or_bool(e, 0))
    }

    fn has_grounded_plain_write(&self, grounded: &AsciiSets) -> bool {
        self.plain_writes
            .iter()
            .any(|e| is_known_ascii_expr(e, grounded, 0))
    }
}

#[derive(Default)]
struct AsciiSets {
    scalars: BTreeSet<String>,
    objs: BTreeSet<String>,
}

impl AsciiSets {
    fn get(&self, kind: TrackKind) -> &BTreeSet<String> {
        match kind {
            TrackKind::Scalar => &self.scalars,
            TrackKind::Obj => &self.objs,
        }
    }

    fn get_mut(&mut self, kind: TrackKind) -> &mut BTreeSet<String> {
        match kind {
            TrackKind::Scalar => &mut self.scalars,
            TrackKind::Obj => &mut self.objs,
        }
    }
}

#[derive(Default)]
struct TrackedInfos {
    scalars: BTreeMap<String, AsciiTrackInfo>,
    objs: BTreeMap<String, AsciiTrackInfo>,
}

impl TrackedInfos {
    fn get(&self, kind: TrackKind) -> &BTreeMap<String, AsciiTrackInfo> {
        match kind {
            TrackKind::Scalar => &self.scalars,
            TrackKind::Obj => &self.objs,
        }
    }

    fn initial_candidates(&self, kind: TrackKind) -> BTreeSet<String> {
        self.get(kind)
            .iter()
            .filter(|(_, info)| !info.bad && !info.plain_writes.is_empty())
            .map(|(n, _)| n.clone())
            .collect()
    }
}

fn int_range(e: &Expr, depth: u8) -> Option<(i64, i64)> {
    if depth > 6 {
        return None;
    }
    match e {
        Expr::Num(n)
            if n.is_finite()
                && n.fract() == 0.0
                && *n >= i64::MIN as f64
                && *n <= i64::MAX as f64 =>
        {
            let v = *n as i64;
            Some((v, v))
        }
        Expr::Binary(jsast::BinOp::BitAnd, l, r) => {
            let lr = int_range(l, depth + 1);
            let rr = int_range(r, depth + 1);
            match (lr, rr) {
                (Some((a, b)), Some((c, d)))
                    if a >= 0 && c >= 0 && b <= 0x7fff_ffff && d <= 0x7fff_ffff =>
                {
                    Some((0, b.min(d)))
                }
                (_, Some((c, d))) if c >= 0 && d <= 0x7fff_ffff => Some((0, d)),
                (Some((a, b)), _) if a >= 0 && b <= 0x7fff_ffff => Some((0, b)),
                _ => None,
            }
        }
        Expr::Binary(jsast::BinOp::UShr, l, r) => {
            let (k, k2) = int_range(r, depth + 1)?;
            if k != k2 || !(0..=31).contains(&k) {
                return None;
            }
            let (lo, hi) = int_range(l, depth + 1)?;
            if lo >= 0 && hi <= 0xffff_ffff {
                Some((lo >> k, hi >> k))
            } else {
                None
            }
        }
        Expr::Binary(jsast::BinOp::Add, l, r) => {
            let (a, b) = int_range(l, depth + 1)?;
            let (c, d) = int_range(r, depth + 1)?;
            Some((a.checked_add(c)?, b.checked_add(d)?))
        }
        Expr::Binary(jsast::BinOp::Rem, l, r) => {
            let (m, m2) = int_range(r, depth + 1)?;
            if m != m2 || m < 1 {
                return None;
            }
            if int_range(l, depth + 1).is_some_and(|(lo, _)| lo >= 0) {
                Some((0, m - 1))
            } else {
                Some((-m, m))
            }
        }
        Expr::Cond(_, t, f) => {
            let (a, b) = int_range(t, depth + 1)?;
            let (c, d) = int_range(f, depth + 1)?;
            Some((a.min(c), b.max(d)))
        }
        _ => None,
    }
}

fn is_ascii_code(e: &Expr) -> bool {
    int_range(e, 0).is_some_and(|(lo, hi)| lo >= 0 && hi <= 127)
}

fn is_numeric_or_bool(e: &Expr, depth: u8) -> bool {
    if depth > 6 {
        return false;
    }
    match e {
        Expr::Num(_) | Expr::RawNum(_) | Expr::Bool(_) | Expr::BigInt(_) => true,
        Expr::Unary(
            jsast::UnOp::Neg | jsast::UnOp::Pos | jsast::UnOp::Not | jsast::UnOp::BitNot,
            _,
        ) => true,
        Expr::Binary(op, l, r) => match op {
            jsast::BinOp::Add | jsast::BinOp::And | jsast::BinOp::Or => {
                is_numeric_or_bool(l, depth + 1) && is_numeric_or_bool(r, depth + 1)
            }
            _ => true,
        },
        Expr::Call(callee, _) => match callee.as_ref() {
            Expr::Ident(n) => n == "Number",
            Expr::Member(obj, _) => matches!(obj.as_ref(), Expr::Ident(n) if n == "Math"),
            _ => false,
        },
        Expr::Cond(_, t, f) => is_numeric_or_bool(t, depth + 1) && is_numeric_or_bool(f, depth + 1),
        _ => false,
    }
}

fn is_known_ascii_expr(e: &Expr, known: &AsciiSets, depth: u8) -> bool {
    if depth > 10 {
        return false;
    }
    match e {
        Expr::Str(s) => s.bytes().all(|b| b < 128),
        Expr::Ident(n) => known.scalars.contains(n),
        Expr::Member(obj, prop) if prop == "s" => {
            matches!(obj.as_ref(), Expr::Ident(n) if known.objs.contains(n))
        }
        Expr::Member(obj, prop) if prop == "text" => {
            if let Expr::Call(callee, _) = obj.as_ref() {
                matches!(
                    rt_member(callee),
                    Some("native_f64_to_string" | "native_f32_to_string")
                )
            } else {
                false
            }
        }
        Expr::Binary(jsast::BinOp::Add, l, r) => {
            let la = is_known_ascii_expr(l, known, depth + 1);
            let ra = is_known_ascii_expr(r, known, depth + 1);
            (la && (ra || is_numeric_or_bool(r, 0))) || (ra && is_numeric_or_bool(l, 0))
        }
        Expr::Cond(_, t, f) => {
            is_known_ascii_expr(t, known, depth + 1) && is_known_ascii_expr(f, known, depth + 1)
        }
        Expr::Seq(items) => items
            .last()
            .is_some_and(|last| is_known_ascii_expr(last, known, depth + 1)),
        Expr::Call(callee, args) => is_known_ascii_call(callee, args, known, depth),
        _ => false,
    }
}

fn is_known_ascii_call(callee: &Expr, args: &[Expr], known: &AsciiSets, depth: u8) -> bool {
    if let Some(member) = rt_member(callee) {
        return match member {
            "fmt_int_dec" | "fmt_int_hex" | "fmt_float_fixed" => true,
            "bstr_out" | "bstr_in" | "bstr_sub" | "bstr_sub_from" | "bstr_sub_to" | "bstr_trim"
            | "bstr_trim_start" | "bstr_trim_end" | "bstr_to_lowercase" | "bstr_to_uppercase" => {
                args.first()
                    .is_some_and(|a| is_known_ascii_expr(a, known, depth + 1))
            }
            _ => false,
        };
    }
    match callee {
        Expr::Ident(n) if n == "String" && args.len() == 1 => {
            is_numeric_or_bool(&args[0], 0) || is_known_ascii_expr(&args[0], known, depth + 1)
        }
        Expr::Member(obj, method) => match (obj.as_ref(), method.as_str()) {
            (Expr::Ident(n), "fromCharCode") if n == "String" => args.iter().all(is_ascii_code),
            (recv, "toString")
                if args.is_empty() || (args.len() == 1 && is_numeric_or_bool(&args[0], 0)) =>
            {
                is_numeric_or_bool(recv, 0) || is_known_ascii_expr(recv, known, depth + 1)
            }
            (
                recv,
                "slice" | "substring" | "repeat" | "toLowerCase" | "toUpperCase" | "trim"
                | "trimStart" | "trimEnd",
            ) => is_known_ascii_expr(recv, known, depth + 1),
            _ => false,
        },
        _ => false,
    }
}

fn match_nstr_obj_init(e: &Expr) -> Option<&Expr> {
    let Expr::Object(entries) = e else {
        return None;
    };
    if entries.len() != 2 {
        return None;
    }
    let mut s_val = None;
    let mut has_cap = false;
    for (k, v) in entries {
        if k == "s" {
            s_val = Some(v);
        } else if k == "cap" {
            has_cap = true;
        }
    }
    has_cap.then_some(s_val?)
}

struct ObjEscapeMarker<'a> {
    locals: &'a BTreeSet<String>,
    objs: &'a mut BTreeMap<String, AsciiTrackInfo>,
}

impl Visit for ObjEscapeMarker<'_> {
    fn visit_expr(&mut self, e: &Expr) {
        match e {
            Expr::Ident(n) if self.locals.contains(n) => {
                self.objs.entry(n.clone()).or_default().bad = true;
            }
            Expr::Arrow(_, _) => {}
            _ => walk_expr(self, e),
        }
    }
}

struct AsciiCollector<'a> {
    locals: &'a BTreeSet<String>,
    infos: TrackedInfos,
}

impl AsciiCollector<'_> {
    fn record_init(&mut self, n: &str, init: &Expr) {
        if self.locals.contains(n) {
            self.infos
                .scalars
                .entry(n.to_owned())
                .or_default()
                .plain_writes
                .push(init.clone());
            if let Some(s_init) = match_nstr_obj_init(init) {
                self.infos
                    .objs
                    .entry(n.to_owned())
                    .or_default()
                    .plain_writes
                    .push(s_init.clone());
            } else {
                self.infos.objs.entry(n.to_owned()).or_default().bad = true;
            }
        }
    }

    fn mark_obj_bad_in_expr(&mut self, e: &Expr) {
        ObjEscapeMarker {
            locals: self.locals,
            objs: &mut self.infos.objs,
        }
        .visit_expr(e);
    }

    fn visit_assign(&mut self, lhs: &Expr, rhs: &Expr) {
        match lhs {
            Expr::Ident(n) if self.locals.contains(n) => {
                self.record_init(n, rhs);
                self.visit_expr(rhs);
            }
            Expr::Member(obj, prop) => {
                if let Expr::Ident(n) = obj.as_ref() {
                    if self.locals.contains(n) {
                        if prop == "s" {
                            self.infos
                                .objs
                                .entry(n.clone())
                                .or_default()
                                .plain_writes
                                .push(rhs.clone());
                            self.visit_expr(rhs);
                            return;
                        }
                        if prop == "cap" {
                            self.visit_expr(rhs);
                            return;
                        }
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            _ => {
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
        }
    }

    fn visit_compound_assign(&mut self, op: jsast::CompoundOp, lhs: &Expr, rhs: &Expr) {
        match lhs {
            Expr::Ident(n) if self.locals.contains(n) => {
                self.infos.objs.entry(n.clone()).or_default().bad = true;
                if op == jsast::CompoundOp::Add {
                    self.infos
                        .scalars
                        .entry(n.clone())
                        .or_default()
                        .append_writes
                        .push(rhs.clone());
                } else {
                    self.infos.scalars.entry(n.clone()).or_default().bad = true;
                }
                self.visit_expr(rhs);
            }
            Expr::Member(obj, prop) => {
                if let Expr::Ident(n) = obj.as_ref() {
                    if self.locals.contains(n) && prop == "s" {
                        if op == jsast::CompoundOp::Add {
                            self.infos
                                .objs
                                .entry(n.clone())
                                .or_default()
                                .append_writes
                                .push(rhs.clone());
                        } else {
                            self.infos.objs.entry(n.clone()).or_default().bad = true;
                        }
                        self.visit_expr(rhs);
                        return;
                    }
                    if self.locals.contains(n) && prop == "cap" {
                        self.visit_expr(rhs);
                        return;
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            _ => {
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
        }
    }
}

impl Visit for AsciiCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(n, Some(init)) | Stmt::Const(n, init) => {
                self.record_init(n, init);
                self.visit_expr(init);
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    if let Some(init) = init {
                        self.record_init(n, init);
                        self.visit_expr(init);
                    }
                }
            }
            Stmt::TryCatch {
                body,
                binding,
                handler,
            } => {
                if self.locals.contains(binding) {
                    self.infos.scalars.entry(binding.clone()).or_default().bad = true;
                    self.infos.objs.entry(binding.clone()).or_default().bad = true;
                }
                self.visit_stmts(body);
                self.visit_stmts(handler);
            }
            Stmt::FunctionDecl { .. } => {}
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, e: &Expr) {
        match e {
            Expr::Ident(n) if self.locals.contains(n) => {
                // Standalone read of `o` (not `o.s` / `o.cap` / `nstr_reserve(o, ...)`) escapes `o`.
                self.infos.objs.entry(n.clone()).or_default().bad = true;
            }
            Expr::Member(obj, prop) => {
                if let Expr::Ident(n) = obj.as_ref() {
                    if self.locals.contains(n) && (prop == "s" || prop == "cap") {
                        return;
                    }
                }
                self.visit_expr(obj);
            }
            Expr::Assign(lhs, rhs) => self.visit_assign(lhs, rhs),
            Expr::CompoundAssign(op, lhs, rhs) => self.visit_compound_assign(*op, lhs, rhs),
            Expr::Call(callee, args) => {
                if rt_member(callee) == Some("nstr_reserve") && args.len() == 2 {
                    if let Expr::Ident(n) = &args[0] {
                        if self.locals.contains(n) {
                            self.visit_expr(&args[1]);
                            return;
                        }
                    }
                }
                walk_expr(self, e);
            }
            Expr::Arrow(_, ArrowBody::Expr(b)) => self.mark_obj_bad_in_expr(b),
            Expr::Arrow(_, ArrowBody::Block(_)) => {}
            _ => walk_expr(self, e),
        }
    }
}

fn solve_known_ascii(infos: &TrackedInfos) -> AsciiSets {
    // Phase A: Greatest fixpoint — shrink optimistic sets until every write to every retained local
    // is proven pure ASCII.
    let mut known = AsciiSets {
        scalars: infos.initial_candidates(TrackKind::Scalar),
        objs: infos.initial_candidates(TrackKind::Obj),
    };
    loop {
        let mut changed = false;
        for kind in TRACK_KINDS {
            let map = infos.get(kind);
            let bad: Vec<String> = known
                .get(kind)
                .iter()
                .filter(|n| !map[*n].all_writes_ascii(&known))
                .cloned()
                .collect();
            for n in bad {
                known.get_mut(kind).remove(&n);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Phase B: Least fixpoint for groundedness from non-cyclic constructive ASCII roots so
    // ungrounded self-cycles (`x = y; y = x;`) cannot claim KnownAscii.
    let mut grounded = AsciiSets::default();
    loop {
        let mut changed = false;
        for kind in TRACK_KINDS {
            let map = infos.get(kind);
            let newly_grounded: Vec<String> = known
                .get(kind)
                .iter()
                .filter(|n| {
                    !grounded.get(kind).contains(*n) && map[*n].has_grounded_plain_write(&grounded)
                })
                .cloned()
                .collect();
            for n in newly_grounded {
                grounded.get_mut(kind).insert(n);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    grounded
}

struct AsciiRewriter<'a> {
    known: &'a AsciiSets,
}

impl VisitMut for AsciiRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr_mut(self, expr);
        let Expr::Call(callee, args) = expr else {
            return;
        };
        let Some(member) = rt_member(callee) else {
            return;
        };
        match (member, args.len()) {
            ("bstr_out" | "bstr_in", 1) if is_known_ascii_expr(&args[0], self.known, 0) => {
                *expr = args.pop().expect("one argument");
            }
            ("_bstr_is_ascii", 1) if is_known_ascii_expr(&args[0], self.known, 0) => {
                let arg = args.pop().expect("one argument");
                *expr = if jsast::is_droppable(&arg) {
                    Expr::Bool(true)
                } else {
                    Expr::Seq(vec![arg, Expr::Bool(true)])
                };
            }
            ("_bstr_is_ascii_range", 3) if is_known_ascii_expr(&args[0], self.known, 0) => {
                let mut seq = Vec::new();
                for a in args.drain(..) {
                    if !jsast::is_droppable(&a) {
                        seq.push(a);
                    }
                }
                *expr = if seq.is_empty() {
                    Expr::Bool(true)
                } else {
                    seq.push(Expr::Bool(true));
                    Expr::Seq(seq)
                };
            }
            ("bstr_to_lowercase", 1) if is_known_ascii_expr(&args[0], self.known, 0) => {
                let arg = args.pop().expect("one argument");
                *expr = jsast::method_call(arg, "toLowerCase", Vec::new());
            }
            ("bstr_to_uppercase", 1) if is_known_ascii_expr(&args[0], self.known, 0) => {
                let arg = args.pop().expect("one argument");
                *expr = jsast::method_call(arg, "toUpperCase", Vec::new());
            }
            _ => {}
        }
    }
}

/// Intraprocedural JS-AST `KnownAscii` pass over a function body: proves local scalar strings and
/// local native `{ s, cap }` `String` objects contain only ASCII bytes (`0..128`), and elides
/// `__rt.bstr_out(e)` / `__rt.bstr_in(e)` / `__rt._bstr_is_ascii(e)` / `__rt.bstr_to_{lower,upper}case(e)`.
///
/// Returns the set of scalar local variable names proven `KnownAscii` for downstream passes.
pub(in crate::lower::bstr) fn elide_known_ascii_bstr_out(body: &mut [Stmt]) -> BTreeSet<String> {
    let mut locals = BTreeSet::new();
    collect_fn_locals(body, &mut locals);
    let mut captured = BTreeSet::new();
    collect_closure_captured(body, &locals, &mut captured);

    let mut collector = AsciiCollector {
        locals: &locals,
        infos: TrackedInfos::default(),
    };
    collector.visit_stmts(body);
    let mut infos = collector.infos;
    for c in &captured {
        infos.scalars.entry(c.clone()).or_default().bad = true;
        infos.objs.entry(c.clone()).or_default().bad = true;
    }

    let grounded = solve_known_ascii(&infos);
    AsciiRewriter { known: &grounded }.visit_stmts_mut(body);
    grounded.scalars
}
