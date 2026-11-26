//! Loop-invariant property hoisting (LICM across cold diverging calls).

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp, Visit, VisitMut};

use super::super::ast_util::{
    collect_written_idents_expr, collect_written_idents_stmts_opt as collect_written_idents_stmts,
    exact_u32, expr_has_side_effect, stmts_definitely_diverge,
};
use super::super::post_opt::alloc_fresh_local;
use super::unroll::detect_loop_counter;
use super::util::{
    collect_any_prop_name_writes_expr, collect_any_prop_name_writes_stmts, expr_has_non_panic_call,
    extract_iter0_then_body_mut, stmts_definitely_exit_loop, stmts_have_mutating_nested_fn,
    stmts_have_non_panic_call,
};

#[derive(Clone, Default)]
struct LicmState {
    non_null: HashSet<String>,
    non_null_props: HashSet<(String, String)>,
    prop_aliases: HashMap<String, (String, String)>,
}

impl LicmState {
    fn invalidate_ident(&mut self, w: &str) {
        self.non_null.remove(w);
        self.prop_aliases.remove(w);
        self.prop_aliases.retain(|_, (base, _)| base != w);
        self.non_null_props.retain(|(base, _)| base != w);
    }

    fn invalidate_prop_name(&mut self, prop: &str) {
        self.prop_aliases.retain(|_, (_, p)| p != prop);
        self.non_null_props.retain(|(_, p)| p != prop);
    }

    fn invalidate_calls(&mut self) {
        self.prop_aliases.clear();
        self.non_null_props.clear();
    }

    fn invalidate_effects(
        &mut self,
        written: impl IntoIterator<Item = String>,
        prop_writes: impl IntoIterator<Item = String>,
        has_call: bool,
    ) {
        for w in written {
            self.invalidate_ident(&w);
        }
        for p in prop_writes {
            self.invalidate_prop_name(&p);
        }
        if has_call {
            self.invalidate_calls();
        }
    }

    fn record_unconditional_expr(&mut self, expr: &Expr) {
        let mut bases = HashSet::new();
        UncondPropVisitor {
            bases: &mut bases,
            nested: Some(&mut self.non_null_props),
        }
        .visit_expr(expr);
        for b in bases {
            if let Some((orig_obj, orig_prop)) = self.prop_aliases.get(&b).cloned() {
                self.non_null_props.insert((orig_obj, orig_prop));
            }
            self.non_null.insert(b);
        }
        if expr_has_non_panic_call(expr) {
            self.invalidate_calls();
        }
        let mut prop_w = HashSet::new();
        collect_prop_writes_expr(expr, &mut prop_w);
        for (_, p) in prop_w {
            self.invalidate_prop_name(&p);
        }
    }
}

struct HoistPropsCx<'a> {
    local_names: &'a mut HashSet<String>,
    extra_decls: &'a mut Vec<(String, Option<Expr>)>,
    escaped_locals: &'a HashSet<String>,
    immutable_slices: &'a HashSet<String>,
}

struct LoopHoistAnalysis {
    assigned_in_loop: HashSet<String>,
    props_written_in_loop: HashSet<(String, String)>,
    any_prop_name_written: HashSet<String>,
    field_prop_reads: HashMap<(String, String), usize>,
    has_non_panic: bool,
    loop_non_null: HashSet<String>,
}

pub(in super::super) fn hoist_loop_invariant_props(
    stmts: &mut Vec<Stmt>,
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
    immutable_slices: &HashSet<String>,
) {
    if stmts_have_mutating_nested_fn(stmts) {
        return;
    }
    let mut escaped_locals: HashSet<String> = HashSet::new();
    collect_escaped_idents_stmts(stmts, &mut escaped_locals);
    let mut state = LicmState::default();
    state.non_null.extend(immutable_slices.iter().cloned());
    let mut cx = HoistPropsCx {
        local_names,
        extra_decls,
        escaped_locals: &escaped_locals,
        immutable_slices,
    };
    hoist_props_in_block(stmts, &mut cx, &mut state);
}

fn hoist_props_in_block(stmts: &mut Vec<Stmt>, cx: &mut HoistPropsCx<'_>, state: &mut LicmState) {
    let mut i = 0;
    while i < stmts.len() {
        if stmts[i].as_loop().is_some() {
            let spliced = hoist_loop_props(stmts, i, cx, state);
            i += spliced + 1;
        } else {
            hoist_props_in_stmt(&mut stmts[i], cx, state);
            i += 1;
        }
    }
}

fn record_binding_init(name: &str, init: Option<&Expr>, state: &mut LicmState) {
    if let Some(rhs) = init {
        state.record_unconditional_expr(rhs);
        let guaranteed_nn = is_guaranteed_non_null_rhs(rhs);
        state.invalidate_ident(name);
        if guaranteed_nn {
            state.non_null.insert(name.to_owned());
        }
    } else {
        state.invalidate_ident(name);
    }
}

fn hoist_props_if(
    cond: &Expr,
    a: &mut Vec<Stmt>,
    b: &mut Option<Vec<Stmt>>,
    cx: &mut HoistPropsCx<'_>,
    state: &mut LicmState,
) {
    state.record_unconditional_expr(cond);
    let mut a_st = state.clone();
    hoist_props_in_block(a, cx, &mut a_st);
    if let Some(b) = b {
        let mut b_st = state.clone();
        hoist_props_in_block(b, cx, &mut b_st);
    }
    let mut written = HashSet::new();
    let mut prop_w = HashSet::new();
    collect_written_idents_stmts(a, &mut written, true);
    collect_any_prop_name_writes_stmts(a, &mut prop_w);
    let mut has_call = stmts_have_non_panic_call(a);
    if let Some(b) = b {
        collect_written_idents_stmts(b, &mut written, true);
        collect_any_prop_name_writes_stmts(b, &mut prop_w);
        has_call |= stmts_have_non_panic_call(b);
    }
    state.invalidate_effects(written, prop_w, has_call);
}

fn hoist_props_switch(
    disc: &Expr,
    cases: &mut [jsast::SwitchCase],
    cx: &mut HoistPropsCx<'_>,
    state: &mut LicmState,
) {
    state.record_unconditional_expr(disc);
    let mut written = HashSet::new();
    let mut prop_w = HashSet::new();
    let mut has_call = false;
    for c in cases.iter_mut() {
        let mut c_st = state.clone();
        hoist_props_in_block(&mut c.body, cx, &mut c_st);
        collect_written_idents_stmts(&c.body, &mut written, true);
        collect_any_prop_name_writes_stmts(&c.body, &mut prop_w);
        has_call |= stmts_have_non_panic_call(&c.body);
    }
    state.invalidate_effects(written, prop_w, has_call);
}

fn hoist_props_in_stmt(stmt: &mut Stmt, cx: &mut HoistPropsCx<'_>, state: &mut LicmState) {
    match stmt {
        Stmt::If(cond, a, b) => hoist_props_if(cond, a, b, cx, state),
        Stmt::Switch(disc, cases) => hoist_props_switch(disc, cases, cx, state),
        Stmt::Block(b) => hoist_props_in_block(b, cx, state),
        Stmt::TryCatch { body, handler, .. } => {
            let mut b_st = state.clone();
            hoist_props_in_block(body, cx, &mut b_st);
            let mut written = HashSet::new();
            collect_written_idents_stmts(body, &mut written, false);
            state.invalidate_effects(written, std::iter::empty(), true);
            let mut h_st = state.clone();
            hoist_props_in_block(handler, cx, &mut h_st);
            let mut h_written = HashSet::new();
            collect_written_idents_stmts(handler, &mut h_written, false);
            state.invalidate_effects(h_written, std::iter::empty(), true);
        }
        Stmt::Labeled(_, inner) => match inner.as_mut() {
            Stmt::Block(b) => {
                let mut written = HashSet::new();
                let mut prop_w = HashSet::new();
                collect_written_idents_stmts(b, &mut written, false);
                collect_any_prop_name_writes_stmts(b, &mut prop_w);
                let has_call = stmts_have_non_panic_call(b);
                let mut b_st = state.clone();
                hoist_props_in_block(b, cx, &mut b_st);
                state.invalidate_effects(written, prop_w, has_call);
            }
            Stmt::Switch(disc, cases) => hoist_props_switch(disc, cases, cx, state),
            Stmt::If(cond, a, b) => hoist_props_if(cond, a, b, cx, state),
            _ => {
                let mut written = HashSet::new();
                collect_written_idents_stmts(
                    std::slice::from_ref(inner.as_ref()),
                    &mut written,
                    true,
                );
                state.invalidate_effects(written, std::iter::empty(), true);
            }
        },
        Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
            state.record_unconditional_expr(rhs);
            if let Expr::Ident(dst) = lhs.as_ref() {
                let dst_clone = dst.clone();
                let alias = match rhs.as_ref() {
                    Expr::Member(base, prop) => match base.as_ref() {
                        Expr::Ident(b) => Some((b.clone(), prop.clone())),
                        _ => None,
                    },
                    _ => None,
                };
                let guaranteed_nn = is_guaranteed_non_null_rhs(rhs)
                    || alias
                        .as_ref()
                        .is_some_and(|pair| state.non_null_props.contains(pair));
                state.invalidate_ident(&dst_clone);
                if guaranteed_nn {
                    state.non_null.insert(dst_clone.clone());
                }
                if let Some((b, p)) = alias {
                    if b != dst_clone {
                        state.prop_aliases.insert(dst_clone, (b, p));
                    }
                }
            } else {
                state.record_unconditional_expr(lhs);
                if let Expr::Member(_, prop) = lhs.as_ref() {
                    state.invalidate_prop_name(prop);
                }
            }
        }
        Stmt::Let(n, init) => record_binding_init(n, init.as_ref(), state),
        Stmt::Const(n, rhs) => record_binding_init(n, Some(rhs), state),
        Stmt::Lets(decls) => {
            for (n, init) in decls {
                record_binding_init(n, init.as_ref(), state);
            }
        }
        Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
            state.record_unconditional_expr(e);
            let mut written = HashSet::new();
            collect_written_idents_expr(e, &mut written);
            for w in written {
                state.invalidate_ident(&w);
            }
        }
        _ => {}
    }
}

fn analyze_loop_hoist_candidates(
    stmts: &[Stmt],
    loop_idx: usize,
    local_names: &HashSet<String>,
    state: &LicmState,
) -> LoopHoistAnalysis {
    let mut assigned_in_loop = HashSet::new();
    let mut props_written_in_loop = HashSet::new();
    let mut any_prop_name_written = HashSet::new();
    let mut field_prop_reads: HashMap<(String, String), usize> = HashMap::new();
    let lp = stmts[loop_idx].as_loop().expect("loop");
    if let Some(c) = lp.cond {
        collect_written_idents_expr(c, &mut assigned_in_loop);
        collect_prop_writes_expr(c, &mut props_written_in_loop);
        collect_any_prop_name_writes_expr(c, &mut any_prop_name_written);
        collect_field_prop_reads_expr(c, &mut field_prop_reads);
    }
    collect_written_idents_stmts(lp.body, &mut assigned_in_loop, true);
    collect_prop_writes_stmts(lp.body, &mut props_written_in_loop);
    collect_any_prop_name_writes_stmts(lp.body, &mut any_prop_name_written);
    collect_field_prop_reads_stmts(lp.body, &mut field_prop_reads);
    let has_non_panic =
        lp.cond.is_some_and(expr_has_non_panic_call) || stmts_have_non_panic_call(lp.body);

    let mut loop_non_null = state.non_null.clone();
    // Any object unconditionally dereferenced on loop entry (e.g. `while (g < a.len)` or
    // leading straight-line statements of `for (;;)`) is non-null on entry.
    if let Some(c) = lp.cond {
        collect_first_eval_prop_bases_expr(c, local_names, &mut loop_non_null);
    } else {
        for s in lp.body {
            match s {
                Stmt::ExprStmt(e) => {
                    if !collect_first_eval_prop_bases_expr(e, local_names, &mut loop_non_null) {
                        break;
                    }
                }
                Stmt::If(c, _, _) => {
                    collect_first_eval_prop_bases_expr(c, local_names, &mut loop_non_null);
                    break;
                }
                _ => break,
            }
        }
    }
    if let Some(c) = detect_loop_counter(stmts, loop_idx, &assigned_in_loop, None) {
        if let (Some(c_start), Some(c_end)) = (c.iv_min, exact_u32(&c.bound)) {
            if c_start < c_end {
                let iter0_body: Option<&[Stmt]> = if lp.cond.is_some() {
                    Some(lp.body)
                } else {
                    match lp.body {
                        [Stmt::If(_, then_b, _)] | [_, Stmt::If(_, then_b, _)] => {
                            Some(then_b.as_slice())
                        }
                        _ => None,
                    }
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

    LoopHoistAnalysis {
        assigned_in_loop,
        props_written_in_loop,
        any_prop_name_written,
        field_prop_reads,
        has_non_panic,
        loop_non_null,
    }
}

/// Round 1 (call-free loops only): hoist repeated invariant container fields (`a.matrix`,
/// `a.query`) and propagate straight-line loop-local aliases (`G = $hp_0; G.buf` -> `$hp_0.buf`)
/// so Round 2 can hoist the underlying `.buf`/`.off`/`.len` lanes out of the loop too.
fn hoist_loop_field_props(
    loop_stmt: &mut Stmt,
    analysis: &mut LoopHoistAnalysis,
    cx: &mut HoistPropsCx<'_>,
    state: &LicmState,
    pre_hoisted_stmts: &mut Vec<Stmt>,
) {
    if analysis.has_non_panic {
        return;
    }
    let mut field_cands: Vec<(String, String)> = analysis
        .field_prop_reads
        .drain()
        .filter_map(|((obj, prop), count)| {
            if count < 2
                || analysis.assigned_in_loop.contains(&obj)
                || analysis.any_prop_name_written.contains(&prop)
                || !analysis.loop_non_null.contains(&obj)
            {
                return None;
            }
            Some((obj, prop))
        })
        .collect();
    field_cands.sort();
    if !field_cands.is_empty() {
        let mut f_subst: HashMap<(String, String), String> = HashMap::new();
        for (obj, prop) in field_cands {
            let tmp = alloc_fresh_local(cx.local_names, cx.extra_decls, "$hp_");
            pre_hoisted_stmts.push(jsast::assign_stmt(
                jsast::id(&tmp),
                Expr::Member(Box::new(Expr::Ident(obj.clone())), prop.clone()),
            ));
            if state.non_null_props.contains(&(obj.clone(), prop.clone())) {
                analysis.loop_non_null.insert(tmp.clone());
            }
            f_subst.insert((obj, prop), tmp);
        }
        replace_hoisted_props_stmt(loop_stmt, &f_subst);
    }
    if let Some(lp) = loop_stmt.as_loop_mut() {
        let mut aliases = HashMap::new();
        propagate_loop_slice_aliases_stmts(lp.body, &analysis.assigned_in_loop, &mut aliases);
    }
}

type PropPairList = Vec<(String, String)>;

fn find_hoistable_props(
    analysis: &LoopHoistAnalysis,
    prop_reads: HashMap<(String, String), usize>,
    iter0_bases: Option<&HashSet<String>>,
    cx: &HoistPropsCx<'_>,
) -> (PropPairList, PropPairList) {
    let mut uncond_cands: Vec<(String, String)> = Vec::new();
    let mut guarded_cands: Vec<(String, String)> = Vec::new();
    for ((obj, prop), _count) in prop_reads {
        if !is_slice_prop(&prop)
            || analysis.assigned_in_loop.contains(&obj)
            || analysis
                .props_written_in_loop
                .contains(&(obj.clone(), prop.clone()))
        {
            continue;
        }
        if analysis.any_prop_name_written.contains(&prop)
            && cx.escaped_locals.contains(&obj)
            && !cx.immutable_slices.contains(&obj)
        {
            continue;
        }
        if analysis.has_non_panic
            && !cx.immutable_slices.contains(&obj)
            && (!cx.local_names.contains(&obj) || cx.escaped_locals.contains(&obj))
        {
            continue;
        }
        if analysis.loop_non_null.contains(&obj) {
            uncond_cands.push((obj, prop));
        } else if iter0_bases.is_some_and(|bases| bases.contains(&obj)) {
            guarded_cands.push((obj, prop));
        }
    }
    uncond_cands.sort();
    guarded_cands.sort();
    (uncond_cands, guarded_cands)
}

fn slice_prop_prefix(prop: &str) -> &'static str {
    match prop {
        "buf" => "$hb_",
        "off" => "$ho_",
        _ => "$hl_",
    }
}

/// Round 2: hoist invariant `.buf`/`.off`/`.len` property loads (including single-use loads
/// inside panic-free hot loops like `fflate::framing::gzip_sync`).
fn hoist_loop_slice_props(
    loop_stmt: &mut Stmt,
    analysis: &LoopHoistAnalysis,
    cx: &mut HoistPropsCx<'_>,
    pre_hoisted_stmts: &mut Vec<Stmt>,
) {
    let mut prop_reads: HashMap<(String, String), usize> = HashMap::new();
    let lp = loop_stmt.as_loop().expect("loop");
    if let Some(c) = lp.cond {
        collect_slice_prop_reads_expr(c, &mut prop_reads);
    }
    collect_slice_prop_reads_stmts(lp.body, &mut prop_reads);

    let iter0_guard = if !analysis.has_non_panic {
        extract_iter0_guard_and_bases(loop_stmt)
    } else {
        None
    };

    let (uncond_cands, guarded_cands) = find_hoistable_props(
        analysis,
        prop_reads,
        iter0_guard.as_ref().map(|(_, bases)| bases),
        cx,
    );

    if !uncond_cands.is_empty() {
        let mut subst: HashMap<(String, String), String> = HashMap::new();
        for (obj, prop) in uncond_cands {
            let tmp = alloc_fresh_local(cx.local_names, cx.extra_decls, slice_prop_prefix(&prop));
            pre_hoisted_stmts.push(jsast::assign_stmt(
                jsast::id(&tmp),
                Expr::Member(Box::new(Expr::Ident(obj.clone())), prop.clone()),
            ));
            subst.insert((obj, prop), tmp);
        }
        replace_hoisted_props_stmt(loop_stmt, &subst);
    }

    if !guarded_cands.is_empty() {
        if let Some((guard_cond, _)) = iter0_guard {
            let mut g_stmts = Vec::with_capacity(guarded_cands.len());
            let mut g_subst: HashMap<(String, String), String> = HashMap::new();
            for (obj, prop) in guarded_cands {
                let tmp =
                    alloc_fresh_local(cx.local_names, cx.extra_decls, slice_prop_prefix(&prop));
                g_stmts.push(jsast::assign_stmt(
                    jsast::id(&tmp),
                    Expr::Member(Box::new(Expr::Ident(obj.clone())), prop.clone()),
                ));
                g_subst.insert((obj, prop), tmp);
            }
            if let Some(then_body) = extract_iter0_then_body_mut(loop_stmt) {
                for s in then_body.iter_mut() {
                    replace_hoisted_props_stmt(s, &g_subst);
                }
                pre_hoisted_stmts.push(Stmt::If(guard_cond, g_stmts, None));
            }
        }
    }
}

fn hoist_loop_props(
    stmts: &mut Vec<Stmt>,
    i: usize,
    cx: &mut HoistPropsCx<'_>,
    state: &mut LicmState,
) -> usize {
    // First recurse into nested loops (innermost loops first!).
    if let Some(lp) = stmts[i].as_loop_mut() {
        let mut written = HashSet::new();
        let mut prop_w = HashSet::new();
        if let Some(cond) = lp.cond.as_deref() {
            collect_written_idents_expr(cond, &mut written);
            collect_any_prop_name_writes_expr(cond, &mut prop_w);
        }
        collect_written_idents_stmts(lp.body, &mut written, true);
        collect_any_prop_name_writes_stmts(lp.body, &mut prop_w);
        let has_call = lp.cond.as_deref().is_some_and(expr_has_non_panic_call)
            || stmts_have_non_panic_call(lp.body);
        let mut inner_st = state.clone();
        inner_st.invalidate_effects(written, prop_w, has_call);
        hoist_props_in_block(lp.body, cx, &mut inner_st);
    }

    let mut analysis = analyze_loop_hoist_candidates(stmts, i, cx.local_names, state);
    let mut pre_hoisted_stmts: Vec<Stmt> = Vec::new();
    hoist_loop_field_props(
        &mut stmts[i],
        &mut analysis,
        cx,
        state,
        &mut pre_hoisted_stmts,
    );
    hoist_loop_slice_props(&mut stmts[i], &analysis, cx, &mut pre_hoisted_stmts);

    let spliced = pre_hoisted_stmts.len();
    if spliced > 0 {
        stmts.splice(i..i, pre_hoisted_stmts);
    }

    state.invalidate_effects(
        analysis.assigned_in_loop,
        analysis.any_prop_name_written,
        analysis.has_non_panic,
    );
    spliced
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

struct UncondPropVisitor<'a> {
    bases: &'a mut HashSet<String>,
    nested: Option<&'a mut HashSet<(String, String)>>,
}

impl Visit for UncondPropVisitor<'_> {
    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Member(obj, _) => {
                match obj.as_ref() {
                    Expr::Ident(id) => {
                        self.bases.insert(id.clone());
                    }
                    Expr::Member(inner_obj, inner_prop) => {
                        if let Some(nested) = self.nested.as_deref_mut() {
                            if let Expr::Ident(id) = inner_obj.as_ref() {
                                nested.insert((id.clone(), inner_prop.clone()));
                            }
                        }
                    }
                    _ => {}
                }
                self.visit_expr(obj);
            }
            Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                self.visit_expr(r);
                if !matches!(l.as_ref(), Expr::Ident(_)) {
                    self.visit_expr(l);
                }
            }
            Expr::Binary(op, l, r) => {
                self.visit_expr(l);
                if !matches!(op, BinOp::And | BinOp::Or) {
                    self.visit_expr(r);
                }
            }
            Expr::Cond(c, _, _) => self.visit_expr(c),
            Expr::Arrow(..) => {}
            _ => jsast::walk_expr(self, expr),
        }
    }
}

fn collect_unconditional_prop_bases_expr(expr: &Expr, out: &mut HashSet<String>) {
    UncondPropVisitor {
        bases: out,
        nested: None,
    }
    .visit_expr(expr);
}

fn is_hoistable_field_prop(prop: &str) -> bool {
    !is_slice_prop(prop)
        && prop != "length"
        && !prop.is_empty()
        && prop
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
        && !prop.as_bytes()[0].is_ascii_digit()
}

struct PropReadCollector<'a> {
    pred: fn(&str) -> bool,
    out: &'a mut HashMap<(String, String), usize>,
}

impl Visit for PropReadCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::If(c, a, b) => {
                self.visit_expr(c);
                if !stmts_definitely_diverge(a) {
                    self.visit_stmts(a);
                }
                if let Some(b) = b {
                    if !stmts_definitely_diverge(b) {
                        self.visit_stmts(b);
                    }
                }
            }
            Stmt::Switch(d, cases) => {
                self.visit_expr(d);
                for c in cases {
                    self.visit_stmts(&c.body);
                }
            }
            Stmt::FunctionDecl { .. } => {}
            _ => jsast::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Arrow(..) => {}
            Expr::Member(obj, prop) if (self.pred)(prop) => {
                if let Expr::Ident(id) = obj.as_ref() {
                    *self.out.entry((id.clone(), prop.clone())).or_default() += 1;
                }
                self.visit_expr(obj);
            }
            Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                if let Expr::Member(base, _) = lhs.as_ref() {
                    self.visit_expr(base);
                } else {
                    self.visit_expr(lhs);
                }
                self.visit_expr(rhs);
            }
            _ => jsast::walk_expr(self, expr),
        }
    }
}

fn collect_field_prop_reads_expr(expr: &Expr, out: &mut HashMap<(String, String), usize>) {
    PropReadCollector {
        pred: is_hoistable_field_prop,
        out,
    }
    .visit_expr(expr);
}

fn collect_field_prop_reads_stmts(stmts: &[Stmt], out: &mut HashMap<(String, String), usize>) {
    PropReadCollector {
        pred: is_hoistable_field_prop,
        out,
    }
    .visit_stmts(stmts);
}

fn collect_slice_prop_reads_expr(expr: &Expr, out: &mut HashMap<(String, String), usize>) {
    PropReadCollector {
        pred: is_slice_prop,
        out,
    }
    .visit_expr(expr);
}

fn collect_slice_prop_reads_stmts(stmts: &[Stmt], out: &mut HashMap<(String, String), usize>) {
    PropReadCollector {
        pred: is_slice_prop,
        out,
    }
    .visit_stmts(stmts);
}

/// Propagates straight-line loop-local invariant aliases (`G = $hp_0; an = G.buf, ao = G.len;` ->
/// `an = $hp_0.buf, ao = $hp_0.len;`) so a 2nd-level slice-prop LICM pass sees invariant bases.
fn propagate_loop_slice_aliases_expr(expr: &mut Expr, aliases: &HashMap<String, String>) {
    struct Rewriter<'a>(&'a HashMap<String, String>);
    impl VisitMut for Rewriter<'_> {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if let Expr::Member(obj, prop) = expr {
                if is_slice_prop(prop) {
                    if let Expr::Ident(id) = obj.as_mut() {
                        if let Some(inv) = self.0.get(id) {
                            *id = inv.clone();
                            return;
                        }
                    }
                }
            }
            match expr {
                Expr::Arrow(..) => {}
                Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Member(base, _) = lhs.as_mut() {
                        self.visit_expr_mut(base);
                    } else {
                        self.visit_expr_mut(lhs);
                    }
                    self.visit_expr_mut(rhs);
                }
                _ => jsast::walk_expr_mut(self, expr),
            }
        }
    }
    Rewriter(aliases).visit_expr_mut(expr);
}

fn propagate_loop_slice_aliases_stmts(
    stmts: &mut [Stmt],
    assigned_in_loop: &HashSet<String>,
    aliases: &mut HashMap<String, String>,
) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
                propagate_loop_slice_aliases_expr(rhs, aliases);
                if let Expr::Ident(dst) = lhs.as_ref() {
                    let dst = dst.clone();
                    aliases.remove(&dst);
                    if let Expr::Ident(src) = rhs.as_ref() {
                        if !assigned_in_loop.contains(src) && src != &dst {
                            aliases.insert(dst, src.clone());
                        }
                    }
                } else {
                    if let Expr::Member(base, _) = lhs.as_mut() {
                        propagate_loop_slice_aliases_expr(base, aliases);
                    } else {
                        propagate_loop_slice_aliases_expr(lhs, aliases);
                    }
                    let mut w = HashSet::new();
                    collect_written_idents_expr(lhs, &mut w);
                    for k in w {
                        aliases.remove(&k);
                    }
                }
            }
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) | Stmt::Const(_, e) => {
                propagate_loop_slice_aliases_expr(e, aliases);
                let mut w = HashSet::new();
                collect_written_idents_expr(e, &mut w);
                for k in w {
                    aliases.remove(&k);
                }
            }
            Stmt::If(c, a, b) => {
                propagate_loop_slice_aliases_expr(c, aliases);
                let mut cw = HashSet::new();
                collect_written_idents_expr(c, &mut cw);
                for k in cw {
                    aliases.remove(&k);
                }
                let mut a_al = aliases.clone();
                propagate_loop_slice_aliases_stmts(a, assigned_in_loop, &mut a_al);
                if let Some(b) = b {
                    let mut b_al = aliases.clone();
                    propagate_loop_slice_aliases_stmts(b, assigned_in_loop, &mut b_al);
                }
                let mut w = HashSet::new();
                collect_written_idents_stmts(a, &mut w, true);
                if let Some(b) = b {
                    collect_written_idents_stmts(b, &mut w, true);
                }
                for k in w {
                    aliases.remove(&k);
                }
            }
            Stmt::Block(b) => {
                propagate_loop_slice_aliases_stmts(b, assigned_in_loop, aliases);
            }
            Stmt::Labeled(_, inner) => {
                if let Stmt::Block(b) = inner.as_mut() {
                    let mut b_al = aliases.clone();
                    propagate_loop_slice_aliases_stmts(b, assigned_in_loop, &mut b_al);
                }
                let mut w = HashSet::new();
                collect_written_idents_stmts(std::slice::from_ref(inner.as_ref()), &mut w, false);
                for k in w {
                    aliases.remove(&k);
                }
            }
            other => {
                let mut w = HashSet::new();
                collect_written_idents_stmts(std::slice::from_ref(other), &mut w, true);
                for k in w {
                    aliases.remove(&k);
                }
            }
        }
    }
}

/// Collects object identifiers whose property is unconditionally evaluated before any possible
/// exception, short-circuit, or side effect when `expr` begins executing. Returns `true` iff
/// evaluating `expr` completed without any possible exception or short-circuit.
fn collect_first_eval_prop_bases_expr(
    expr: &Expr,
    local_names: &HashSet<String>,
    out: &mut HashSet<String>,
) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Null
        | Expr::Undefined => true,
        Expr::Ident(id) => local_names.contains(id),
        Expr::Member(obj, _) => {
            if !collect_first_eval_prop_bases_expr(obj, local_names, out) {
                return false;
            }
            if let Expr::Ident(id) = obj.as_ref() {
                if local_names.contains(id) {
                    if out.contains(id) {
                        return true;
                    }
                    out.insert(id.clone());
                }
            }
            false
        }
        Expr::Index(obj, idx) => {
            if !collect_first_eval_prop_bases_expr(obj, local_names, out) {
                return false;
            }
            collect_first_eval_prop_bases_expr(idx, local_names, out);
            false
        }
        Expr::Unary(_, inner) => collect_first_eval_prop_bases_expr(inner, local_names, out),
        Expr::Binary(op, l, r) => {
            if !collect_first_eval_prop_bases_expr(l, local_names, out) {
                return false;
            }
            if matches!(op, BinOp::And | BinOp::Or) {
                return false;
            }
            collect_first_eval_prop_bases_expr(r, local_names, out)
        }
        Expr::Assign(lhs, rhs) if matches!(lhs.as_ref(), Expr::Ident(id) if local_names.contains(id)) => {
            collect_first_eval_prop_bases_expr(rhs, local_names, out)
        }
        _ => false,
    }
}

fn extract_iter0_guard_and_bases(stmt: &Stmt) -> Option<(Expr, HashSet<String>)> {
    let lp = stmt.as_loop()?;
    let (guard, body0): (Expr, &[Stmt]) = if let Some(c) = lp.cond {
        if expr_has_side_effect(c) {
            return None;
        }
        (c.clone(), lp.body)
    } else {
        match lp.body {
            [Stmt::If(c, then_b, Some(_))] if !expr_has_side_effect(c) => {
                (c.clone(), then_b.as_slice())
            }
            [Stmt::ExprStmt(Expr::Assign(fl, fr)), Stmt::If(Expr::Ident(fu), then_b, Some(_))]
                if matches!(fl.as_ref(), Expr::Ident(fn_) if fn_ == fu)
                    && !expr_has_side_effect(fr) =>
            {
                (fr.as_ref().clone(), then_b.as_slice())
            }
            [Stmt::If(c, exit_b, None), rest @ ..]
                if !rest.is_empty()
                    && !expr_has_side_effect(c)
                    && stmts_definitely_exit_loop(exit_b, lp.label) =>
            {
                let neg = match c {
                    Expr::Unary(UnOp::Not, inner) => (**inner).clone(),
                    other => Expr::Unary(UnOp::Not, Box::new(other.clone())),
                };
                (neg, rest)
            }
            _ => return None,
        }
    };
    let mut bases = HashSet::new();
    for s in body0 {
        match s {
            Stmt::ExprStmt(e) if !expr_has_non_panic_call(e) => {
                collect_unconditional_prop_bases_expr(e, &mut bases);
            }
            Stmt::If(cond, then_b, None)
                if stmts_definitely_diverge(then_b) && !expr_has_non_panic_call(cond) =>
            {
                collect_unconditional_prop_bases_expr(cond, &mut bases);
                break;
            }
            _ => break,
        }
    }
    (!bases.is_empty()).then_some((guard, bases))
}

fn collect_escaped_idents_stmts(stmts: &[Stmt], escaped: &mut HashSet<String>) {
    struct EscapedCollector<'a>(&'a mut HashSet<String>);
    impl Visit for EscapedCollector<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            if !matches!(stmt, Stmt::FunctionDecl { .. }) {
                jsast::walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Member(obj, prop)
                    if is_slice_prop(prop) && matches!(obj.as_ref(), Expr::Ident(_)) => {}
                Expr::Ident(id) => {
                    self.0.insert(id.clone());
                }
                Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                    if !matches!(l.as_ref(), Expr::Ident(_)) {
                        self.visit_expr(l);
                    }
                    if !matches!(r.as_ref(), Expr::Member(o, p) if is_slice_prop(p) && matches!(o.as_ref(), Expr::Ident(_)))
                    {
                        self.visit_expr(r);
                    }
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    if !matches!(c.as_ref(), Expr::Ident(n) if n.starts_with("core$panicking$")) {
                        self.visit_expr(c);
                        for a in args {
                            self.visit_expr(a);
                        }
                    }
                }
                Expr::Arrow(params, ArrowBody::Expr(body)) => {
                    let mut inner = HashSet::new();
                    EscapedCollector(&mut inner).visit_expr(body);
                    for id in inner {
                        if !params.iter().any(|p| p == &id) {
                            self.0.insert(id);
                        }
                    }
                }
                Expr::Arrow(_, ArrowBody::Block(_)) => {}
                _ => jsast::walk_expr(self, expr),
            }
        }
    }
    EscapedCollector(escaped).visit_stmts(stmts);
}

struct PropWriteCollector<'a>(&'a mut HashSet<(String, String)>);
impl Visit for PropWriteCollector<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if !matches!(stmt, Stmt::FunctionDecl { .. }) {
            jsast::walk_stmt(self, stmt);
        }
    }
    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(expr, Expr::Arrow(..)) {
            return;
        }
        if let Expr::Assign(lhs, _) | Expr::CompoundAssign(_, lhs, _) = expr {
            if let Expr::Member(obj, prop) = lhs.as_ref() {
                if let Expr::Ident(id) = obj.as_ref() {
                    self.0.insert((id.clone(), prop.clone()));
                }
            }
        }
        jsast::walk_expr(self, expr);
    }
}

fn collect_prop_writes_expr(expr: &Expr, out: &mut HashSet<(String, String)>) {
    PropWriteCollector(out).visit_expr(expr);
}

fn collect_prop_writes_stmts(stmts: &[Stmt], out: &mut HashSet<(String, String)>) {
    PropWriteCollector(out).visit_stmts(stmts);
}

fn replace_hoisted_props_stmt(stmt: &mut Stmt, subst: &HashMap<(String, String), String>) {
    struct Rewriter<'a>(&'a HashMap<(String, String), String>);
    impl VisitMut for Rewriter<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if !matches!(stmt, Stmt::FunctionDecl { .. }) {
                jsast::walk_stmt_mut(self, stmt);
            }
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if let Expr::Member(obj, prop) = expr {
                if let Expr::Ident(id) = obj.as_ref() {
                    if let Some(tmp) = self.0.get(&(id.clone(), prop.clone())) {
                        *expr = Expr::Ident(tmp.clone());
                        return;
                    }
                }
            }
            match expr {
                Expr::Arrow(..) => {}
                Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                    if let Expr::Member(base, _) = lhs.as_mut() {
                        self.visit_expr_mut(base);
                    } else {
                        self.visit_expr_mut(lhs);
                    }
                    self.visit_expr_mut(rhs);
                }
                _ => jsast::walk_expr_mut(self, expr),
            }
        }
    }
    Rewriter(subst).visit_stmt_mut(stmt);
}
