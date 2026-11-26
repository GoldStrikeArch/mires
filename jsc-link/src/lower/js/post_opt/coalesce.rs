//! Liveness-based local slot coalescing (register coloring for uninitialized `let` locals).

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp, Visit, VisitMut};

use super::super::ast_util::FunctionSafetyScan;

// =============================================================================================
// Dense bitset over candidate local indices
// =============================================================================================

#[derive(Clone, Eq, PartialEq)]
struct LocalBitSet {
    words: Vec<u64>,
}

impl LocalBitSet {
    fn new(bits: usize) -> Self {
        Self {
            words: vec![0; bits.div_ceil(64)],
        }
    }

    fn set(&mut self, idx: usize) {
        self.words[idx >> 6] |= 1u64 << (idx & 63);
    }

    fn clear(&mut self, idx: usize) {
        self.words[idx >> 6] &= !(1u64 << (idx & 63));
    }

    fn contains(&self, idx: usize) -> bool {
        (self.words[idx >> 6] & (1u64 << (idx & 63))) != 0
    }

    fn union_with(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            let next = *a | *b;
            if next != *a {
                *a = next;
                changed = true;
            }
        }
        changed
    }

    fn count_ones(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }

    fn iter_ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &word)| {
            let base = wi << 6;
            BitIter { word, base }
        })
    }
}

struct BitIter {
    word: u64,
    base: usize,
}

impl Iterator for BitIter {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.word == 0 {
            return None;
        }
        let tz = self.word.trailing_zeros() as usize;
        self.word &= self.word - 1;
        Some(self.base + tz)
    }
}

// =============================================================================================
// Candidate collection & interference graph
// =============================================================================================

struct CandidateSet {
    prefix_end: usize,
    candidates: Vec<String>,
    cand_idx: HashMap<String, usize>,
}

fn collect_candidates(
    params: &[String],
    body: &[Stmt],
    min_candidates: usize,
) -> Option<CandidateSet> {
    // 1. Collect leading top-level uninitialized `let` declarations.
    let param_set: HashSet<&str> = params.iter().map(String::as_str).collect();
    let mut prefix_end = 0usize;
    let mut decl_counts: HashMap<String, usize> = HashMap::new();
    let mut init_decls: HashSet<String> = HashSet::new();
    let mut candidate_order: Vec<String> = Vec::new();
    while prefix_end < body.len() {
        match &body[prefix_end] {
            Stmt::Let(n, init) => {
                *decl_counts.entry(n.clone()).or_default() += 1;
                if init.is_some() {
                    init_decls.insert(n.clone());
                } else {
                    candidate_order.push(n.clone());
                }
                prefix_end += 1;
            }
            Stmt::Lets(decls) => {
                for (n, init) in decls {
                    *decl_counts.entry(n.clone()).or_default() += 1;
                    if init.is_some() {
                        init_decls.insert(n.clone());
                    } else {
                        candidate_order.push(n.clone());
                    }
                }
                prefix_end += 1;
            }
            Stmt::Const(n, _) => {
                *decl_counts.entry(n.clone()).or_default() += 1;
                init_decls.insert(n.clone());
                prefix_end += 1;
            }
            _ => break,
        }
    }
    if candidate_order.len() < min_candidates {
        return None;
    }

    // 2. Scan the entire body for disqualifiers: `Raw` JS, inner re-declarations/catch-var
    //    shadowing, or captures inside nested `Arrow` / `FunctionDecl`. Also record any mentions
    //    in the leading initializers (which execute before the main body).
    let mut scan = FunctionSafetyScan {
        in_nested: true,
        ..Default::default()
    };
    for stmt in &body[..prefix_end] {
        match stmt {
            Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => scan.visit_expr(e),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        scan.visit_expr(e);
                    }
                }
            }
            _ => {}
        }
    }
    scan.in_nested = false;
    scan.visit_stmts(&body[prefix_end..]);
    if scan.has_raw {
        return None;
    }

    let mut candidates: Vec<String> = Vec::new();
    let mut cand_idx: HashMap<String, usize> = HashMap::new();
    for name in candidate_order {
        if param_set.contains(name.as_str())
            || init_decls.contains(&name)
            || decl_counts.get(&name).copied().unwrap_or(0) != 1
            || scan.disqualified.contains(&name)
            || scan.declared.contains(&name)
            || !scan.used_outside_nested.contains(&name)
        {
            continue;
        }
        let idx = candidates.len();
        cand_idx.insert(name.clone(), idx);
        candidates.push(name);
    }
    if candidates.len() < min_candidates {
        return None;
    }
    Some(CandidateSet {
        prefix_end,
        candidates,
        cand_idx,
    })
}

struct InterferenceGraph {
    edges: Vec<LocalBitSet>,
}

impl InterferenceGraph {
    fn new(n: usize) -> Self {
        Self {
            edges: vec![LocalBitSet::new(n); n],
        }
    }

    fn add_def(&mut self, v: usize, live: &LocalBitSet, handler_live: &[LocalBitSet]) {
        for u in live.iter_ones() {
            if u != v {
                self.edges[v].set(u);
                self.edges[u].set(v);
            }
        }
        for h in handler_live {
            for u in h.iter_ones() {
                if u != v {
                    self.edges[v].set(u);
                    self.edges[u].set(v);
                }
            }
        }
    }

    fn degree(&self, v: usize) -> u32 {
        self.edges[v].count_ones()
    }

    fn neighbors(&self, v: usize) -> impl Iterator<Item = usize> + '_ {
        self.edges[v].iter_ones()
    }
}

// =============================================================================================
// Backward dataflow liveness analysis
// =============================================================================================

fn restore_label(map: &mut HashMap<String, LocalBitSet>, lbl: &str, prev: Option<LocalBitSet>) {
    match prev {
        Some(p) => {
            map.insert(lbl.to_owned(), p);
        }
        None => {
            map.remove(lbl);
        }
    }
}

struct LivenessScan<'a> {
    cand_idx: &'a HashMap<String, usize>,
    n: usize,
    graph: InterferenceGraph,
    unlabeled_break: Vec<LocalBitSet>,
    unlabeled_continue: Vec<LocalBitSet>,
    labeled_break: HashMap<String, LocalBitSet>,
    labeled_continue: HashMap<String, LocalBitSet>,
    handler_live: Vec<LocalBitSet>,
    bailed: bool,
}

impl<'a> LivenessScan<'a> {
    fn analyze(
        cand_idx: &'a HashMap<String, usize>,
        n: usize,
        stmts: &[Stmt],
    ) -> Option<(LocalBitSet, InterferenceGraph)> {
        let mut scan = Self {
            cand_idx,
            n,
            graph: InterferenceGraph::new(n),
            unlabeled_break: Vec::new(),
            unlabeled_continue: Vec::new(),
            labeled_break: HashMap::new(),
            labeled_continue: HashMap::new(),
            handler_live: Vec::new(),
            bailed: false,
        };
        let live_at_entry = scan.walk_stmts(stmts, LocalBitSet::new(n));
        (!scan.bailed).then_some((live_at_entry, scan.graph))
    }

    fn union_handlers(&self, live: &mut LocalBitSet) {
        for h in &self.handler_live {
            live.union_with(h);
        }
    }

    fn walk_stmts(&mut self, stmts: &[Stmt], mut live: LocalBitSet) -> LocalBitSet {
        for s in stmts.iter().rev() {
            live = self.walk_stmt(s, live, None);
            if !self.handler_live.is_empty() {
                self.union_handlers(&mut live);
            }
            if self.bailed {
                return LocalBitSet::new(self.n);
            }
        }
        live
    }

    fn walk_loop_forever(
        &mut self,
        lbl: Option<&str>,
        stmts: &[Stmt],
        live: LocalBitSet,
    ) -> LocalBitSet {
        self.unlabeled_break.push(live.clone());
        let prev_lb = lbl.and_then(|l| self.labeled_break.insert(l.to_owned(), live.clone()));
        let mut head = LocalBitSet::new(self.n);
        let mut converged = false;
        for _ in 0..16 {
            self.unlabeled_continue.push(head.clone());
            let prev_lc =
                lbl.and_then(|l| self.labeled_continue.insert(l.to_owned(), head.clone()));
            let next_head = self.walk_stmts(stmts, head.clone());
            self.unlabeled_continue.pop();
            if let Some(l) = lbl {
                restore_label(&mut self.labeled_continue, l, prev_lc);
            }
            if self.bailed {
                break;
            }
            if !head.union_with(&next_head) {
                converged = true;
                break;
            }
        }
        self.unlabeled_break.pop();
        if let Some(l) = lbl {
            restore_label(&mut self.labeled_break, l, prev_lb);
        }
        if !converged {
            self.bailed = true;
        }
        head
    }

    fn walk_while(
        &mut self,
        lbl: Option<&str>,
        cond: &Expr,
        stmts: &[Stmt],
        live: LocalBitSet,
    ) -> LocalBitSet {
        self.unlabeled_break.push(live.clone());
        let prev_lb = lbl.and_then(|l| self.labeled_break.insert(l.to_owned(), live.clone()));
        let mut head = live.clone();
        self.walk_expr(cond, &mut head);
        let mut converged = false;
        for _ in 0..16 {
            self.unlabeled_continue.push(head.clone());
            let prev_lc =
                lbl.and_then(|l| self.labeled_continue.insert(l.to_owned(), head.clone()));
            let mut body_in = self.walk_stmts(stmts, head.clone());
            self.unlabeled_continue.pop();
            if let Some(l) = lbl {
                restore_label(&mut self.labeled_continue, l, prev_lc);
            }
            if self.bailed {
                break;
            }
            body_in.union_with(&live);
            self.walk_expr(cond, &mut body_in);
            if !head.union_with(&body_in) {
                converged = true;
                break;
            }
        }
        self.unlabeled_break.pop();
        if let Some(l) = lbl {
            restore_label(&mut self.labeled_break, l, prev_lb);
        }
        if !converged {
            self.bailed = true;
        }
        head
    }

    fn walk_switch(
        &mut self,
        disc: &Expr,
        cases: &[jsast::SwitchCase],
        live: LocalBitSet,
    ) -> LocalBitSet {
        let has_default = cases.iter().any(|c| c.tests.is_empty());
        self.unlabeled_break.push(live.clone());
        let mut fallthrough = live.clone();
        let mut dispatch = if has_default {
            LocalBitSet::new(self.n)
        } else {
            live
        };
        for c in cases.iter().rev() {
            let case_in = self.walk_stmts(&c.body, fallthrough);
            fallthrough = case_in.clone();
            dispatch.union_with(&case_in);
            for t in c.tests.iter().rev() {
                self.walk_expr(t, &mut dispatch);
            }
        }
        self.unlabeled_break.pop();
        self.walk_expr(disc, &mut dispatch);
        dispatch
    }

    fn walk_stmt(
        &mut self,
        stmt: &Stmt,
        mut live: LocalBitSet,
        outer_label: Option<&str>,
    ) -> LocalBitSet {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Let(_, Some(e)) | Stmt::Const(_, e) => {
                self.walk_expr(e, &mut live);
                live
            }
            Stmt::Lets(decls) => {
                for (_, init) in decls.iter().rev() {
                    if let Some(e) = init {
                        self.walk_expr(e, &mut live);
                    }
                }
                live
            }
            Stmt::Return(opt) => {
                let mut out = LocalBitSet::new(self.n);
                if let Some(e) = opt {
                    self.walk_expr(e, &mut out);
                }
                out
            }
            Stmt::Throw(e) => {
                let mut out = LocalBitSet::new(self.n);
                self.union_handlers(&mut out);
                self.walk_expr(e, &mut out);
                out
            }
            Stmt::Break(None) => self.jump_target(self.unlabeled_break.last().cloned()),
            Stmt::Break(Some(lbl)) => self.jump_target(self.labeled_break.get(lbl).cloned()),
            Stmt::Continue(None) => self.jump_target(self.unlabeled_continue.last().cloned()),
            Stmt::Continue(Some(lbl)) => self.jump_target(self.labeled_continue.get(lbl).cloned()),
            Stmt::Block(stmts) => self.walk_stmts(stmts, live),
            Stmt::If(cond, then_b, else_b) => {
                let then_in = self.walk_stmts(then_b, live.clone());
                let else_in = match else_b {
                    Some(eb) => self.walk_stmts(eb, live),
                    None => live,
                };
                let mut merged = then_in;
                merged.union_with(&else_in);
                self.walk_expr(cond, &mut merged);
                merged
            }
            Stmt::Labeled(lbl, inner) => {
                if matches!(
                    inner.as_ref(),
                    Stmt::While(..) | Stmt::LoopForever(None, ..)
                ) {
                    return self.walk_stmt(inner, live, Some(lbl.as_str()));
                }
                let prev = self.labeled_break.insert(lbl.clone(), live.clone());
                let out = self.walk_stmt(inner, live, None);
                restore_label(&mut self.labeled_break, lbl, prev);
                out
            }
            Stmt::LoopForever(own_lbl, stmts) => {
                let lbl = own_lbl.as_deref().or(outer_label);
                self.walk_loop_forever(lbl, stmts, live)
            }
            Stmt::While(cond, stmts) => self.walk_while(outer_label, cond, stmts, live),
            Stmt::Switch(disc, cases) => self.walk_switch(disc, cases, live),
            Stmt::TryCatch {
                body,
                binding: _,
                handler,
            } => {
                let handler_in = self.walk_stmts(handler, live.clone());
                self.handler_live.push(handler_in.clone());
                let mut body_in = self.walk_stmts(body, live);
                self.handler_live.pop();
                body_in.union_with(&handler_in);
                body_in
            }
            Stmt::Let(_, None)
            | Stmt::FunctionDecl { .. }
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Loc(_)
            | Stmt::Raw(_) => live,
        }
    }

    fn jump_target(&mut self, target: Option<LocalBitSet>) -> LocalBitSet {
        match target {
            Some(live) => live,
            None => {
                self.bailed = true;
                LocalBitSet::new(self.n)
            }
        }
    }

    fn walk_expr(&mut self, expr: &Expr, live: &mut LocalBitSet) {
        match expr {
            Expr::Ident(n) => {
                if let Some(&idx) = self.cand_idx.get(n) {
                    live.set(idx);
                }
            }
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if let Some(&idx) = self.cand_idx.get(n) {
                        self.graph.add_def(idx, live, &self.handler_live);
                        live.clear(idx);
                    }
                    self.walk_expr(rhs, live);
                } else {
                    // Non-ident LHS (`arr[i] = rhs`, `obj.p = rhs`): JS evaluates LHS before RHS.
                    self.walk_expr(rhs, live);
                    self.walk_expr(lhs, live);
                }
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if let Some(&idx) = self.cand_idx.get(n) {
                        self.graph.add_def(idx, live, &self.handler_live);
                        self.walk_expr(rhs, live);
                        live.set(idx);
                    } else {
                        self.walk_expr(rhs, live);
                    }
                } else {
                    self.walk_expr(rhs, live);
                    self.walk_expr(lhs, live);
                }
            }
            Expr::Binary(BinOp::And | BinOp::Or, l, r) => {
                let mut r_live = live.clone();
                self.walk_expr(r, &mut r_live);
                live.union_with(&r_live);
                self.walk_expr(l, live);
            }
            Expr::Cond(c, t, e) => {
                let mut t_live = live.clone();
                self.walk_expr(t, &mut t_live);
                let mut e_live = live.clone();
                self.walk_expr(e, &mut e_live);
                *live = t_live;
                live.union_with(&e_live);
                self.walk_expr(c, live);
            }
            Expr::Binary(_, l, r) => {
                self.walk_expr(r, live);
                self.walk_expr(l, live);
            }
            Expr::Index(o, i) => {
                self.walk_expr(i, live);
                self.walk_expr(o, live);
            }
            Expr::Member(o, _) | Expr::Unary(_, o) => self.walk_expr(o, live),
            Expr::Call(c, args) | Expr::New(c, args) => {
                for a in args.iter().rev() {
                    self.walk_expr(a, live);
                }
                self.walk_expr(c, live);
            }
            Expr::Object(entries) => {
                for (_, v) in entries.iter().rev() {
                    self.walk_expr(v, live);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items.iter().rev() {
                    self.walk_expr(x, live);
                }
            }
            Expr::Arrow(..)
            | Expr::Raw(_)
            | Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null => {}
        }
    }
}

// =============================================================================================
// Coarse JS value-bucket classification
// =============================================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValueBucket {
    Unseen,
    Bool,
    Number,
    String,
    SpillErr,
    Other,
}

impl ValueBucket {
    fn join(self, other: Self) -> Self {
        if self == Self::Unseen {
            other
        } else if other == Self::Unseen || self == other {
            self
        } else {
            Self::Other
        }
    }
}

fn expr_bucket(expr: &Expr) -> ValueBucket {
    match expr {
        Expr::Bool(_) => ValueBucket::Bool,
        Expr::Num(_) | Expr::RawNum(_) => ValueBucket::Number,
        Expr::Str(_) => ValueBucket::String,
        Expr::Ident(n) if n.starts_with("$r") => ValueBucket::SpillErr,
        Expr::Unary(UnOp::Not, _) => ValueBucket::Bool,
        Expr::Unary(UnOp::TypeOf, _) => ValueBucket::String,
        Expr::Unary(UnOp::Neg | UnOp::BitNot, _) => ValueBucket::Number,
        Expr::Binary(
            BinOp::StrictEq | BinOp::StrictNe | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
            _,
            _,
        ) => ValueBucket::Bool,
        Expr::Binary(
            BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::Rem
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::Shl
            | BinOp::Shr
            | BinOp::UShr,
            _,
            _,
        ) => ValueBucket::Number,
        Expr::Call(c, _) => {
            if let Expr::Member(o, m) = c.as_ref() {
                if m == "charCodeAt" || matches!(o.as_ref(), Expr::Ident(id) if id == "Math") {
                    return ValueBucket::Number;
                }
                if m == "isArray" || m == "startsWith" || m == "endsWith" {
                    return ValueBucket::Bool;
                }
            } else if let Expr::Ident(id) = c.as_ref() {
                if id == "Number" {
                    return ValueBucket::Number;
                }
                if id == "Boolean" {
                    return ValueBucket::Bool;
                }
            }
            ValueBucket::Other
        }
        Expr::Cond(_, t, e) => expr_bucket(t).join(expr_bucket(e)),
        _ => ValueBucket::Other,
    }
}

struct BucketScan<'a> {
    cand_idx: &'a HashMap<String, usize>,
    buckets: &'a mut [ValueBucket],
}

impl Visit for BucketScan<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if !matches!(stmt, Stmt::FunctionDecl { .. }) {
            jsast::walk_stmt(self, stmt);
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Arrow(..) => {}
            Expr::Assign(lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if let Some(&idx) = self.cand_idx.get(n) {
                        self.buckets[idx] = self.buckets[idx].join(expr_bucket(rhs));
                    }
                } else {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Ident(n) = lhs.as_ref() {
                    if let Some(&idx) = self.cand_idx.get(n) {
                        self.buckets[idx] = self.buckets[idx].join(ValueBucket::Number);
                    }
                } else {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            _ => jsast::walk_expr(self, expr),
        }
    }
}

fn classify_buckets(
    cand_idx: &HashMap<String, usize>,
    n: usize,
    stmts: &[Stmt],
) -> Vec<ValueBucket> {
    let mut buckets = vec![ValueBucket::Unseen; n];
    BucketScan {
        cand_idx,
        buckets: &mut buckets,
    }
    .visit_stmts(stmts);
    for b in &mut buckets {
        if *b == ValueBucket::Unseen {
            *b = ValueBucket::Other;
        }
    }
    buckets
}

// =============================================================================================
// Welsh-Powell bucketed coloring & AST renamer
// =============================================================================================

struct Coalescer<'a> {
    candidates: &'a [String],
    graph: &'a InterferenceGraph,
    buckets: &'a [ValueBucket],
    live_at_entry: &'a LocalBitSet,
}

impl Coalescer<'_> {
    /// Greedy Welsh-Powell graph coloring strictly within each `ValueBucket`, over non-entry-live
    /// candidates. Any variable live at function entry may read its initial `undefined` value, so
    /// keep it pinned in its own dedicated slot.
    fn compute_rename_map(&self) -> HashMap<String, String> {
        let n = self.candidates.len();
        let mut order: Vec<usize> = (0..n)
            .filter(|&i| !self.live_at_entry.contains(i))
            .collect();
        order.sort_by(|&a, &b| {
            self.graph
                .degree(b)
                .cmp(&self.graph.degree(a))
                .then_with(|| a.cmp(&b))
        });

        let mut color_of: Vec<Option<usize>> = vec![None; n];
        let mut color_name: Vec<String> = Vec::new();
        let mut color_bucket: Vec<ValueBucket> = Vec::new();
        let mut used_colors: Vec<usize> = Vec::new();
        for v in order {
            let vb = self.buckets[v];
            used_colors.clear();
            for u in self.graph.neighbors(v) {
                if let Some(c) = color_of[u] {
                    used_colors.push(c);
                }
            }
            let chosen = (0..color_name.len())
                .find(|&c| color_bucket[c] == vb && !used_colors.contains(&c))
                .unwrap_or_else(|| {
                    let c = color_name.len();
                    color_name.push(self.candidates[v].clone());
                    color_bucket.push(vb);
                    c
                });
            color_of[v] = Some(chosen);
        }

        let mut rename: HashMap<String, String> = HashMap::new();
        for (idx, name) in self.candidates.iter().enumerate() {
            if let Some(c) = color_of[idx] {
                let target = &color_name[c];
                if target != name {
                    rename.insert(name.clone(), target.clone());
                }
            }
        }
        rename
    }
}

struct LocalRenamer<'a>(&'a HashMap<String, String>);

impl VisitMut for LocalRenamer<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if !matches!(stmt, Stmt::FunctionDecl { .. }) {
            jsast::walk_stmt_mut(self, stmt);
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Arrow(..) => {}
            Expr::Ident(n) => {
                if let Some(target) = self.0.get(n) {
                    *n = target.clone();
                }
            }
            _ => jsast::walk_expr_mut(self, expr),
        }
    }
}

fn apply_renames_and_prune(
    body: &mut Vec<Stmt>,
    mut prefix_end: usize,
    rename: &HashMap<String, String>,
) {
    LocalRenamer(rename).visit_stmts_mut(&mut body[prefix_end..]);
    let mut i = 0;
    while i < prefix_end {
        let remove = match &mut body[i] {
            Stmt::Lets(decls) => {
                decls.retain(|(n, init)| init.is_some() || !rename.contains_key(n));
                decls.is_empty()
            }
            Stmt::Let(n, None) => rename.contains_key(n),
            _ => false,
        };
        if remove {
            body.remove(i);
            prefix_end -= 1;
        } else {
            i += 1;
        }
    }
}

// =============================================================================================
// Entry points
// =============================================================================================

/// Liveness-based local slot coalescing over a finalized `Stmt::FunctionDecl`.
///
/// Must run AFTER `hoist_consts::run` and `bstr::rewrite` so single-assignment AST analyses
/// (`elide_ascii_only_bstr_in`, `elide_known_ascii_bstr_out`, `elide_proven_bstr_sub`,
/// `find_read_only_const_slice_init`, `split_bigint_props`) see pristine SSA-style locals first.
pub(crate) fn coalesce_disjoint_locals_decl(decl: &mut Stmt) {
    if let Stmt::FunctionDecl {
        name, params, body, ..
    } = decl
    {
        // Only coalesce directly self-recursive functions with huge local frames (>220 uninit
        // temporaries, e.g. `jmespath::eval` after mutual-recursion inlining) so deep recursion
        // stays within V8's stack budget without adding register/type-feedback pressure to
        // non-recursive hot functions (`marked`, `d3-hierarchy`, `diff`).
        if body_calls_self(name, body) {
            coalesce_disjoint_locals(params, body);
        }
    }
}

pub(super) fn body_calls_self(self_name: &str, stmts: &[Stmt]) -> bool {
    jsast::any_expr_in_stmts(stmts, |expr| {
        matches!(
            expr,
            Expr::Call(callee, _) if matches!(callee.as_ref(), Expr::Ident(id) if id == self_name)
        )
    })
}

fn coalesce_disjoint_locals(params: &[String], body: &mut Vec<Stmt>) {
    // Only coalesce directly self-recursive functions with very large local frames (>220 uninit
    // temporaries) to keep deep-recursion V8 stack frames compact.
    coalesce_disjoint_locals_with_threshold(params, body, 221);
}

pub(super) fn coalesce_disjoint_locals_with_threshold(
    params: &[String],
    body: &mut Vec<Stmt>,
    min_candidates: usize,
) {
    let Some(cands) = collect_candidates(params, body, min_candidates) else {
        return;
    };
    let n = cands.candidates.len();
    let Some((live_at_entry, graph)) =
        LivenessScan::analyze(&cands.cand_idx, n, &body[cands.prefix_end..])
    else {
        return;
    };
    let buckets = classify_buckets(&cands.cand_idx, n, &body[cands.prefix_end..]);
    let rename = Coalescer {
        candidates: &cands.candidates,
        graph: &graph,
        buckets: &buckets,
        live_at_entry: &live_at_entry,
    }
    .compute_rename_map();
    if !rename.is_empty() {
        apply_renames_and_prune(body, cands.prefix_end, &rename);
    }
}
