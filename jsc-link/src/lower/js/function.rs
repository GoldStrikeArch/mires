//! One item as a top level declaration: a function (`base.rs::codegen_fn` and `codegen_body`),
//! a `static` (`base.rs::codegen_static`), or a vtable (`vtable.rs`).

use std::collections::{BTreeSet, HashMap, HashSet};

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::{Decline, OpClass};
use rcgjs_object::jir::repr::Func;
use rcgjs_object::jir::semantic::{BodyKind, VtableEntry};
use rcgjs_object::jsast::{self, ArrowBody, Expr, Stmt};
use rcgjs_object::ptr;
use structurizer::Region;

use crate::opts::OptimizerPass;

use super::super::plan::{BlockPayload, Plan, RScrut};
use super::super::{LowerInput, LoweredItem};
use super::{decline, emit, value, JsCx};

/// Lowers a function body whose SSA form, plan and region tree are in hand.
#[allow(dead_code)]
pub fn lower(
    input: &LowerInput<'_>,
    func: &Func,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
) -> Result<LoweredItem, Decline> {
    lower_with_ascii(input, func, plan, region, None)
}

/// [`lower`] carrying whole-program ASCII provenance facts (`js-str=bytes`).
pub fn lower_with_ascii(
    input: &LowerInput<'_>,
    func: &Func,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
    ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'_>>,
) -> Result<LoweredItem, Decline> {
    let physical = input
        .abi
        .and_then(|abi| abi.target_signature(input.name, func.item));
    let cx = JsCx::new(
        func,
        plan,
        input.table,
        input.interns,
        input.opts,
        input.summary,
        input.origins,
    )
    .with_items(input.items)
    .with_drop_glue(input.drop_glue)
    .with_vtables(input.vtables)
    .with_spills(input.spills)
    .with_abi(input.abi)
    .with_physical(input.name, input.physical)
    .with_target_abi(physical)
    .with_call_retargets(input.call_retargets)
    .with_bigint_hoist(input.bigints.is_some())
    .with_byte_views(input.byte_views)
    .with_ascii(ascii);

    // The parameter list (`base.rs::params`): `Func.params` in order — one per MIR argument, a
    // splatted tuple already as its arity, the caller `Location` last when the body tracks it.
    let slice_positions = physical.map_or_else(Vec::new, |sig| sig.slice_jir_positions());
    let tagged_positions = physical.map_or_else(Vec::new, |sig| sig.tagged_jir_positions());
    let recursive_positions = physical.map_or_else(Vec::new, |sig| sig.recursive_jir_positions());
    let i64_positions = physical.map_or_else(Vec::new, |sig| sig.i64_pair_jir_positions());
    for value in &plan.physical_slices {
        let Some(name) = cx.name_of(*value) else {
            return Err(decline(
                OpClass::Other,
                format!("physical slice {} has no name in the plan", value.0),
            ));
        };
        cx.bind_expr(
            *value,
            ptr::fat_slice(
                jsast::id(format!("{name}$buf")),
                jsast::id(format!("{name}$off")),
                jsast::id(format!("{name}$len")),
            ),
        );
    }
    for value in &plan.physical_slots {
        let Some(name) = cx.name_of(*value) else {
            return Err(decline(
                OpClass::Other,
                format!("physical slot {} has no name in the plan", value.0),
            ));
        };
        cx.bind_expr(
            *value,
            ptr::slot(
                jsast::id(format!("{name}$buf")),
                jsast::id(format!("{name}$off")),
            ),
        );
    }
    let mut params: Vec<String> = Vec::with_capacity(func.params.len());
    let mut logical_param_names = Vec::new();
    let mut unpacked_i64_params: Vec<(String, String, String)> = Vec::new();
    for (position, param) in func.params.iter().enumerate() {
        let Some(name) = cx.name_of(*param) else {
            return Err(decline(
                OpClass::Other,
                format!("parameter {} has no name in the plan", param.0),
            ));
        };
        let slice = slice_positions.contains(&position);
        if slice {
            let buf = format!("{name}$buf");
            let off = format!("{name}$off");
            let len = format!("{name}$len");
            params.extend([buf.clone(), off.clone(), len.clone()]);
            logical_param_names.push(name.to_owned());
            cx.bind_expr(
                *param,
                ptr::fat_slice(jsast::id(buf), jsast::id(off), jsast::id(len)),
            );
        } else if i64_positions.contains(&position) {
            let (lo, hi) = super::i64_pair::pair_names(name);
            params.extend([lo.clone(), hi.clone()]);
            logical_param_names.push(name.to_owned());
            cx.bind_pair(*param, jsast::id(lo), jsast::id(hi));
        } else if let Some((_, shape)) = tagged_positions
            .iter()
            .find(|(tagged_position, _)| *tagged_position == position)
        {
            let tag = format!("{name}$tag");
            let payloads: Vec<String> = (0..shape.fields().saturating_sub(1) as usize)
                .map(|lane| format!("{name}$p{lane}"))
                .collect();
            params.push(tag.clone());
            params.extend(payloads.iter().cloned());
            logical_param_names.push(name.to_owned());
            let Some(value) = value::tagged_product_value(
                input.table,
                input.interns,
                shape,
                jsast::id(tag),
                payloads.into_iter().map(jsast::id).collect(),
            ) else {
                return Err(decline(
                    OpClass::Other,
                    format!(
                        "tagged parameter {} has an unprintable product shape",
                        param.0
                    ),
                ));
            };
            cx.bind_expr(*param, value);
        } else if let Some((_, shape)) = recursive_positions
            .iter()
            .find(|(recursive_position, _)| *recursive_position == position)
        {
            let lanes: Vec<String> = (0..shape.fields() as usize)
                .map(|lane| format!("{name}$p{lane}"))
                .collect();
            params.extend(lanes.iter().cloned());
            logical_param_names.push(name.to_owned());
            let Some(value) = value::recursive_product_value(
                input.table,
                input.interns,
                shape,
                lanes.into_iter().map(jsast::id).collect(),
            ) else {
                return Err(decline(
                    OpClass::Other,
                    format!(
                        "recursive parameter {} has an unprintable product shape",
                        param.0
                    ),
                ));
            };
            cx.bind_expr(*param, value);
        } else if cx.is_i64_ty(cx.ty_of(*param)?) {
            let (lo, hi) = super::i64_pair::pair_names(name);
            params.push(name.to_owned());
            cx.bind_pair(*param, jsast::id(lo.clone()), jsast::id(hi.clone()));
            unpacked_i64_params.push((name.to_owned(), lo, hi));
        } else {
            params.push(name.to_owned());
        }
    }
    let formal_param_names: HashSet<String> = params.iter().cloned().collect();
    cx.set_formal_param_names(formal_param_names.clone());
    let mut param_names: HashSet<String> = formal_param_names.clone();
    param_names.extend(logical_param_names);

    // The body first: it decides how many temporaries get hoisted and which names are mentioned.
    let (mut body, structure_temps) = emit::emit(&cx, region)?;
    if let Some(last) = body.iter().rposition(|stmt| !matches!(stmt, Stmt::Loc(_))) {
        if matches!(body[last], Stmt::Return(None)) {
            body.truncate(last);
        }
    }
    let body_mentions_initial = mentioned_names(&body);
    let mut entry_unpacks = Vec::new();
    let mut unpacked_pair_locals = Vec::new();
    for (canonical, lo, hi) in unpacked_i64_params {
        let uses_lo = body_mentions_initial.contains(&lo);
        let uses_hi = body_mentions_initial.contains(&hi);
        if !uses_lo && !uses_hi {
            continue;
        }
        let (lo_expr, hi_expr) = super::i64_pair::unpack_bigint_exprs(jsast::id(canonical));
        if uses_lo {
            entry_unpacks.push(jsast::assign_stmt(jsast::id(lo.clone()), lo_expr));
            unpacked_pair_locals.push(lo);
        }
        if uses_hi {
            entry_unpacks.push(jsast::assign_stmt(jsast::id(hi.clone()), hi_expr));
            unpacked_pair_locals.push(hi);
        }
    }
    if !entry_unpacks.is_empty() {
        entry_unpacks.append(&mut body);
        body = entry_unpacks;
    }
    // The prelude (`base.rs::codegen_body`): every local declared once — the named values, the
    // cells (a boxed one as its empty box; a parameter's cell is bound by the signature and
    // rebound by the entry block), the copy temporaries — then the hoisted and structurizer
    // temporaries.
    let mut declarations: Vec<(String, Option<jsast::Expr>)> = Vec::new();
    let mut declared: HashSet<String> = HashSet::new();
    for v in &plan.declared_order {
        let Some(name) = cx.name_of(*v) else {
            return Err(decline(
                OpClass::Other,
                format!("declared value {} has no name in the plan", v.0),
            ));
        };
        if param_names.contains(name) {
            continue;
        }
        if plan.physical_slices.contains(v) {
            for suffix in ["$buf", "$off", "$len"] {
                let lane = format!("{name}{suffix}");
                if declared.insert(lane.clone()) {
                    declarations.push((lane, None));
                }
            }
        } else if plan.physical_slots.contains(v) {
            for suffix in ["$buf", "$off"] {
                let lane = format!("{name}{suffix}");
                if declared.insert(lane.clone()) {
                    declarations.push((lane, None));
                }
            }
        } else if cx.is_i64_ty(cx.ty_of(*v)?) {
            let (lo, hi) = super::i64_pair::pair_names(name);
            for lane in [lo, hi] {
                if declared.insert(lane.clone()) {
                    declarations.push((lane, None));
                }
            }
        } else if declared.insert(name.to_owned()) {
            declarations.push((name.to_owned(), None));
        }
    }
    for lane in unpacked_pair_locals {
        if declared.insert(lane.clone()) {
            declarations.push((lane, None));
        }
    }
    for (cell, name) in &plan.cell_names {
        if formal_param_names.contains(name) {
            continue;
        }
        if !declared.insert(name.clone()) {
            continue;
        }
        let init = cx.cell_boxed(*cell).then(|| jsast::array(Vec::new()));
        declarations.push((name.clone(), init));
    }
    for name in plan.invoke_names.values().chain(plan.exception_name.iter()) {
        if declared.insert(name.clone()) {
            declarations.push((name.clone(), None));
        }
    }
    for name in &plan.copy_temps {
        if declared.insert(name.clone()) {
            declarations.push((name.clone(), None));
        }
        let (lo, hi) = super::i64_pair::pair_names(name);
        for lane in [lo, hi] {
            if body_mentions_initial.contains(&lane) && declared.insert(lane.clone()) {
                declarations.push((lane, None));
            }
        }
    }
    declarations.extend(cx.temps().into_iter().map(|name| (name, None)));
    declarations.extend(structure_temps.into_iter().map(|name| (name, None)));
    let mut local_names = declared.clone();
    local_names.extend(param_names.iter().cloned());
    if input.opts.applies(OptimizerPass::AstDce) {
        let uninit_locals: HashSet<String> = declarations
            .iter()
            .filter(|(_, init)| init.is_none())
            .map(|(name, _)| name.clone())
            .collect();
        fuse_local_bytes_str(&mut body, &uninit_locals);
        if crate::lower::bstr::enabled() {
            fuse_local_bstr_in_bytes(&mut body, &uninit_locals);
        }
        eliminate_dead_local_writes(&mut body, &local_names);
    }
    if crate::lower::bstr::enabled() {
        fold_bstr_in_bytes_stmts(&mut body);
    }
    let mentioned = mentioned_names(&body);
    if input.opts.queue {
        let scoped = declared_names(&body);
        declarations.retain(|(name, _)| mentioned.contains(name) && !scoped.contains(name));
    }

    let mut out = Vec::new();
    if !declarations.is_empty() {
        out.push(jsast::lets(declarations));
    }
    out.extend(body);

    let decl = jsast::function(input.name.as_str(), params, out);
    let allocation_origins = cx
        .take_allocation_origins()
        .into_iter()
        .map(|(value, expression)| {
            crate::analysis::js_alloc::Origin::of(expression, value, func, input.interns, plan)
        })
        .collect();
    finish(
        input,
        decl,
        plan.source_names.clone(),
        cx.take_zombies(),
        allocation_origins,
        input.bigints,
    )
}

/// A `static`: `let NAME = <value>`, boxed as `[value]` when the type is direct
/// (`base.rs::codegen_static`).
pub fn lower_static(input: &LowerInput<'_>, plan: &Plan) -> Result<LoweredItem, Decline> {
    let BodyKind::Static { init, .. } = &input.body.kind else {
        return Err(decline(OpClass::Static, "not a static"));
    };
    let func = empty_func(input);
    let cx = JsCx::new(
        &func,
        plan,
        input.table,
        input.interns,
        input.opts,
        input.summary,
        input.origins,
    )
    .with_items(input.items)
    .with_drop_glue(input.drop_glue)
    .with_vtables(input.vtables)
    .with_spills(input.spills)
    .with_abi(input.abi)
    .with_call_retargets(input.call_retargets);
    let ty = input.body.signature.result;
    let value = cx.const_expr(init, ty)?;
    let value = if input.table.is_indirect(ty) {
        value
    } else {
        jsast::array(vec![value])
    };
    let decl = jsast::let_(input.name.as_str(), value);
    // A static's initializer runs once, while the module evaluates, so naming its BigInt constants
    // would buy nothing at run time and cost a declaration per value (`lower/bigint.rs`).
    finish(input, decl, Vec::new(), cx.take_zombies(), Vec::new(), None)
}

/// A vtable: `const NAME = [<drop or null>, <size>, <align>, <method>, ..]` (`vtable.rs`).
pub fn lower_vtable(input: &LowerInput<'_>) -> Result<LoweredItem, Decline> {
    let BodyKind::Vtable {
        concrete, entries, ..
    } = &input.body.kind
    else {
        return Err(decline(OpClass::Vtable, "not a vtable"));
    };
    let table = input.table;
    let (size, align) = match table.get(*concrete) {
        Some(descriptor) => (
            descriptor.layout.size.unwrap_or(0),
            descriptor.layout.align.unwrap_or(1),
        ),
        None => (0, 1),
    };
    let name_of = |id| input.interns.str(id).unwrap_or("").to_owned();
    let slots = entries
        .iter()
        .map(|entry| match entry {
            VtableEntry::Drop(None) | VtableEntry::Vacant => jsast::null(),
            VtableEntry::Drop(Some(glue)) => jsast::id(name_of(glue.link_name)),
            VtableEntry::Size(size) => jsast::num(*size as f64),
            VtableEntry::Align(align) => jsast::num(*align as f64),
            VtableEntry::Method(method) => jsast::id(name_of(method.link_name)),
            VtableEntry::TraitVPtr(name) => jsast::id(name_of(*name)),
        })
        .collect();
    let _ = (size, align);
    let decl = jsast::const_(input.name.as_str(), jsast::array(slots));
    finish(input, decl, Vec::new(), Vec::new(), Vec::new(), None)
}

/// A function with no blocks, for the contexts that lower a static (constants only).
fn empty_func(input: &LowerInput<'_>) -> Func {
    Func {
        item: input.body.item,
        sig: input.body.signature.clone(),
        params: Vec::new(),
        values: Vec::new(),
        cells: Vec::new(),
        blocks: Vec::new(),
    }
}

/// The `LoweredItem` around a finished declaration: refs by `visit_free_idents`, the names table
/// led by the item's own debug path, the root location. `bigints` names the declaration's BigInt
/// constants when it is a function lowered for phase C on the ES2018 target.
fn finish(
    input: &LowerInput<'_>,
    decl: Stmt,
    originals: Vec<(String, String)>,
    zombies: Vec<rcgjs_object::item::FlatZombie>,
    js_alloc_origins: Vec<crate::analysis::js_alloc::Origin>,
    bigints: Option<&crate::lower::bigint::BigIntNames>,
) -> Result<LoweredItem, Decline> {
    // A baseline item passes through the shared finished-JavaScript SROA and DCE pipeline in
    // `JsItem::new`. JIR replacements are installed directly by the linker, so run the same
    // passes here: the semantic optimizer has already done the typed work, but final structuring
    // can expose one last object literal that is only read field by field.
    let decl = rcgjs_object::sroa::analyze_jir(decl, input.opts.optimizer).decl;
    let mut decl = rcgjs_object::optimizer::analyze(decl, input.opts.optimizer).decl;
    // On the ES2018 target a BigInt constant is a call that parses its digits whenever it runs, so
    // a function's final lowering names a module-level binding instead (`lower/bigint.rs`). Last,
    // so the passes above see the constants the trials priced, and before the refs are taken, so
    // the bindings are references like any other. When `OptimizerMode::On` is active, `lane.rs`
    // runs `hoist_consts::run` (including `split_bigint_props`) first and invokes
    // `lower::bigint::hoist` immediately afterwards so struct-field BigInt literals are split
    // before remaining BigInt constants are hoisted into load-safe `$BI_*` bindings.
    let mut hoisted = BTreeSet::new();
    if input.opts.optimizer != crate::opts::OptimizerMode::On {
        if let Some(names) = bigints {
            crate::lower::bigint::hoist(&mut decl, names, &mut hoisted);
        }
    }
    let mut refs = BTreeSet::new();
    jsast::visit_free_idents(&decl, &mut |ident| {
        if ident != input.name.as_str() {
            refs.insert(JsName::new(ident.to_owned()));
        }
    });
    let debug_path = input
        .origins
        .and_then(|origins| input.interns.str(origins.item_path))
        .unwrap_or("")
        .to_owned();
    let mut names = vec![(input.name.as_str().to_owned(), debug_path)];
    names.extend(originals);
    let loc = if input.opts.source_map {
        input
            .origins
            .and_then(|origins| origins.root)
            .and_then(|fact| input.interns.source_fact(fact))
            .and_then(|fact| {
                let file = input.interns.str(fact.file)?;
                if file.is_empty() {
                    return None;
                }
                Some(jsast::Loc {
                    file: file.to_owned(),
                    line: fact.line,
                    col: fact.col,
                })
            })
    } else {
        None
    };
    Ok(LoweredItem {
        decl,
        refs,
        names,
        loc,
        // The TypeScript signature needs the type walk `dts::signature(tcx, ..)` ran; the
        // driver keeps the baseline's when this is `None`.
        dts: None,
        zombies,
        js_alloc_origins,
        bigints: hoisted,
    })
}

/// The names a finished body declares for itself (`base.rs::declared_names`).
fn declared_names(body: &[Stmt]) -> HashSet<String> {
    body.iter()
        .filter_map(|stmt| match stmt {
            Stmt::Const(name, _) | Stmt::Let(name, _) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Every identifier a finished body mentions (`base.rs::mentioned_names`).
fn mentioned_names(body: &[Stmt]) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in body {
        jsast::visit_idents(stmt, &mut |name| {
            names.insert(name.to_string());
        });
    }
    names
}

fn seq_tail(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Seq(items) => items.last(),
        other => Some(other),
    }
}

fn str_view_call_target(expr: &Expr) -> Option<(&'static str, &str)> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    let [Expr::Ident(arg)] = args.as_slice() else {
        return None;
    };
    let Expr::Member(obj, method) = callee.as_ref() else {
        return None;
    };
    if !matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt") {
        return None;
    }
    match method.as_str() {
        "str_bytes" => Some(("str_bytes", arg.as_str())),
        "str_len" => Some(("str_len", arg.as_str())),
        _ => None,
    }
}

fn str_bytes_call_target(expr: &Expr) -> Option<(&'static str, &str)> {
    let Expr::Ident(arg) = rt_unary_call(expr, "str_bytes")? else {
        return None;
    };
    Some(("str_bytes", arg.as_str()))
}

struct LocalFusePolicy {
    peel_rhs: fn(&Expr) -> bool,
    read_call: fn(&Expr) -> Option<(&'static str, &str)>,
    allow_early_exit: bool,
}

fn count_reads_in_expr(expr: &Expr, target: &str, policy: &LocalFusePolicy) -> usize {
    if let Some((_, arg)) = (policy.read_call)(expr) {
        return usize::from(arg == target);
    }
    match expr {
        Expr::Member(obj, _) | Expr::Unary(_, obj) => count_reads_in_expr(obj, target, policy),
        Expr::Index(obj, idx)
        | Expr::Binary(_, obj, idx)
        | Expr::Assign(obj, idx)
        | Expr::CompoundAssign(_, obj, idx) => {
            count_reads_in_expr(obj, target, policy) + count_reads_in_expr(idx, target, policy)
        }
        Expr::Call(c, args) | Expr::New(c, args) => {
            count_reads_in_expr(c, target, policy)
                + args
                    .iter()
                    .map(|a| count_reads_in_expr(a, target, policy))
                    .sum::<usize>()
        }
        Expr::Cond(c, t, e) => {
            count_reads_in_expr(c, target, policy)
                + count_reads_in_expr(t, target, policy)
                + count_reads_in_expr(e, target, policy)
        }
        Expr::Object(entries) => entries
            .iter()
            .map(|(_, v)| count_reads_in_expr(v, target, policy))
            .sum(),
        Expr::Array(items) | Expr::Seq(items) => items
            .iter()
            .map(|x| count_reads_in_expr(x, target, policy))
            .sum(),
        _ => 0,
    }
}

fn count_reads_in_stmts(stmts: &[Stmt], target: &str, policy: &LocalFusePolicy) -> usize {
    let mut total = 0;
    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e)
            | Stmt::Throw(e)
            | Stmt::Const(_, e)
            | Stmt::Let(_, Some(e))
            | Stmt::Return(Some(e)) => total += count_reads_in_expr(e, target, policy),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        total += count_reads_in_expr(e, target, policy);
                    }
                }
            }
            Stmt::If(test, a, b) => {
                total += count_reads_in_expr(test, target, policy)
                    + count_reads_in_stmts(a, target, policy)
                    + b.as_deref()
                        .map_or(0, |s| count_reads_in_stmts(s, target, policy));
            }
            Stmt::Switch(d, cases) => {
                total += count_reads_in_expr(d, target, policy);
                for c in cases {
                    total += c
                        .tests
                        .iter()
                        .map(|t| count_reads_in_expr(t, target, policy))
                        .sum::<usize>();
                    total += count_reads_in_stmts(&c.body, target, policy);
                }
            }
            Stmt::Labeled(_, inner) => {
                total += count_reads_in_stmts(std::slice::from_ref(inner.as_ref()), target, policy);
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                total += count_reads_in_stmts(b, target, policy);
            }
            Stmt::While(t, b) => {
                total += count_reads_in_expr(t, target, policy)
                    + count_reads_in_stmts(b, target, policy);
            }
            Stmt::TryCatch { body, handler, .. } => {
                total += count_reads_in_stmts(body, target, policy)
                    + count_reads_in_stmts(handler, target, policy);
            }
            _ => {}
        }
    }
    total
}

fn stmt_writes_target(stmt: &Stmt, target: &str, policy: &LocalFusePolicy) -> bool {
    match stmt {
        Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => {
            matches!(lhs.as_ref(), Expr::Ident(n) if n == target) && (policy.peel_rhs)(rhs)
        }
        Stmt::Let(name, Some(init)) => name == target && (policy.peel_rhs)(init),
        Stmt::Lets(decls) => decls
            .iter()
            .any(|(name, init)| name == target && init.as_ref().is_some_and(policy.peel_rhs)),
        _ => false,
    }
}

fn expr_is_pure_until_last_str_bytes(
    expr: &Expr,
    target: &str,
    remaining: &mut usize,
    policy: &LocalFusePolicy,
) -> bool {
    if *remaining == 0 {
        return true;
    }
    if let Some((method, arg)) = (policy.read_call)(expr) {
        if arg == target {
            if method == "str_bytes" {
                *remaining = remaining.saturating_sub(1);
            }
            return true;
        }
    }
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::BigInt(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null
        | Expr::Ident(_)
        | Expr::Arrow(..) => true,
        Expr::Member(obj, _) | Expr::Unary(_, obj) => {
            expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
        }
        Expr::Index(obj, idx) => {
            expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
                && expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
        }
        Expr::Binary(op, obj, idx) => {
            if !expr_is_pure_until_last_str_bytes(obj, target, remaining, policy) {
                return false;
            }
            if *remaining == 0 {
                return true;
            }
            if !policy.allow_early_exit && matches!(op, jsast::BinOp::And | jsast::BinOp::Or) {
                // Short-circuit RHS is not guaranteed to run `bstr_utf8_bytes`'s TypeError check.
                let before = *remaining;
                return expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
                    && *remaining == before;
            }
            expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
        }
        Expr::Cond(c, t, e) => {
            if !expr_is_pure_until_last_str_bytes(c, target, remaining, policy) {
                return false;
            }
            if *remaining == 0 {
                return true;
            }
            // Disallow impure calls/writes on either arm while str_bytes reads remain.
            let mut rem_t = *remaining;
            let mut rem_e = *remaining;
            let ok_t = expr_is_pure_until_last_str_bytes(t, target, &mut rem_t, policy);
            let ok_e = expr_is_pure_until_last_str_bytes(e, target, &mut rem_e, policy);
            if !policy.allow_early_exit {
                // With strict validation preservation, neither arm may conditionally consume the
                // target read before it runs unconditionally later.
                return ok_t && ok_e && rem_t == *remaining && rem_e == *remaining;
            }
            *remaining = (*remaining).saturating_sub((*remaining - rem_t) + (*remaining - rem_e));
            ok_t && ok_e && *remaining == 0
        }
        Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
            let lhs_pure = match lhs.as_ref() {
                Expr::Ident(_) => true,
                Expr::Member(obj, _) => {
                    expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
                }
                Expr::Index(obj, idx) => {
                    expr_is_pure_until_last_str_bytes(obj, target, remaining, policy)
                        && expr_is_pure_until_last_str_bytes(idx, target, remaining, policy)
                }
                _ => false,
            };
            if !lhs_pure {
                return false;
            }
            if !expr_is_pure_until_last_str_bytes(rhs, target, remaining, policy) {
                return false;
            }
            // A Member/Index store executes after evaluating lhs and rhs; unsafe if any
            // str_bytes(target) still runs later.
            !matches!(lhs.as_ref(), Expr::Member(..) | Expr::Index(..)) || *remaining == 0
        }
        Expr::Call(callee, args) | Expr::New(callee, args) => {
            if !expr_is_pure_until_last_str_bytes(callee, target, remaining, policy) {
                return false;
            }
            for a in args {
                if !expr_is_pure_until_last_str_bytes(a, target, remaining, policy) {
                    return false;
                }
            }
            *remaining == 0
        }
        Expr::Object(entries) => entries
            .iter()
            .all(|(_, v)| expr_is_pure_until_last_str_bytes(v, target, remaining, policy)),
        Expr::Array(items) | Expr::Seq(items) => items
            .iter()
            .all(|x| expr_is_pure_until_last_str_bytes(x, target, remaining, policy)),
        Expr::Raw(_) => false,
    }
}

fn stmts_is_pure_until_last_str_bytes(
    stmts: &[Stmt],
    target: &str,
    remaining: &mut usize,
    policy: &LocalFusePolicy,
) -> bool {
    for stmt in stmts {
        if *remaining == 0 {
            return true;
        }
        let ok = match stmt {
            Stmt::ExprStmt(e) | Stmt::Const(_, e) | Stmt::Let(_, Some(e)) => {
                expr_is_pure_until_last_str_bytes(e, target, remaining, policy)
            }
            Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                expr_is_pure_until_last_str_bytes(e, target, remaining, policy)
                    && (policy.allow_early_exit || *remaining == 0)
            }
            Stmt::Let(_, None) | Stmt::Import { .. } | Stmt::Comment(_) | Stmt::Loc(_) => true,
            Stmt::Return(None) | Stmt::Break(_) | Stmt::Continue(_) => policy.allow_early_exit,
            Stmt::Lets(decls) => decls.iter().all(|(_, init)| {
                init.as_ref()
                    .is_none_or(|e| expr_is_pure_until_last_str_bytes(e, target, remaining, policy))
            }),
            Stmt::Block(b) => stmts_is_pure_until_last_str_bytes(b, target, remaining, policy),
            Stmt::Labeled(_, inner) => stmts_is_pure_until_last_str_bytes(
                std::slice::from_ref(inner.as_ref()),
                target,
                remaining,
                policy,
            ),
            Stmt::If(..)
            | Stmt::Switch(..)
            | Stmt::LoopForever(..)
            | Stmt::While(..)
            | Stmt::TryCatch { .. }
            | Stmt::FunctionDecl { .. }
            | Stmt::Raw(_) => false,
        };
        if !ok {
            return false;
        }
    }
    *remaining == 0
}

fn write_dominates_and_safe(
    stmts: &[Stmt],
    target: &str,
    total_reads: usize,
    str_bytes_reads: usize,
    policy: &LocalFusePolicy,
) -> Option<bool> {
    for (idx, stmt) in stmts.iter().enumerate() {
        if stmt_writes_target(stmt, target, policy) {
            let mut remaining = str_bytes_reads;
            let mut pre_reads = 0usize;
            if !policy.allow_early_exit {
                if let Stmt::Lets(decls) = stmt {
                    if let Some(pos) = decls.iter().position(|(name, init)| {
                        name == target && init.as_ref().is_some_and(policy.peel_rhs)
                    }) {
                        for (_, init) in &decls[pos + 1..] {
                            if let Some(init_expr) = init {
                                pre_reads += count_reads_in_expr(init_expr, target, policy);
                                if !expr_is_pure_until_last_str_bytes(
                                    init_expr,
                                    target,
                                    &mut remaining,
                                    policy,
                                ) {
                                    return Some(false);
                                }
                            }
                        }
                    }
                }
            }
            let tail = &stmts[idx + 1..];
            if pre_reads + count_reads_in_stmts(tail, target, policy) != total_reads {
                return Some(false);
            }
            if str_bytes_reads == 0 {
                return Some(true);
            }
            return Some(stmts_is_pure_until_last_str_bytes(
                tail,
                target,
                &mut remaining,
                policy,
            ));
        }
        let nested = match stmt {
            Stmt::If(_, a, b) => write_dominates_and_safe(
                a,
                target,
                total_reads,
                str_bytes_reads,
                policy,
            )
            .or_else(|| {
                b.as_deref().and_then(|s| {
                    write_dominates_and_safe(s, target, total_reads, str_bytes_reads, policy)
                })
            }),
            Stmt::Switch(_, cases) => cases.iter().find_map(|c| {
                write_dominates_and_safe(&c.body, target, total_reads, str_bytes_reads, policy)
            }),
            Stmt::Labeled(_, inner) => write_dominates_and_safe(
                std::slice::from_ref(inner.as_ref()),
                target,
                total_reads,
                str_bytes_reads,
                policy,
            ),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                write_dominates_and_safe(b, target, total_reads, str_bytes_reads, policy)
            }
            Stmt::TryCatch { body, handler, .. } => {
                write_dominates_and_safe(body, target, total_reads, str_bytes_reads, policy)
                    .or_else(|| {
                        write_dominates_and_safe(
                            handler,
                            target,
                            total_reads,
                            str_bytes_reads,
                            policy,
                        )
                    })
            }
            _ => None,
        };
        if nested.is_some() {
            return nested;
        }
    }
    None
}

/// Eliminates intra-function `let x; x = (..., __rt.bytes_str(b, o, l))` round-trips when every
/// read of `x` is `__rt.str_bytes(x)` or `__rt.str_len(x)`, rewriting `x` to hold
/// `{ buf: b, off: o, len: l }`, `__rt.str_bytes(x)` to `x`, and `__rt.str_len(x)` to `x.len`.
pub(super) fn fuse_local_bytes_str(body: &mut [Stmt], locals: &HashSet<String>) {
    #[derive(Default)]
    struct LocalUsage {
        writes: usize,
        reads: usize,
        str_bytes_reads: usize,
        disqualified: bool,
    }

    fn peel_bytes_str(expr: &Expr) -> bool {
        let Some(tail) = seq_tail(expr) else {
            return false;
        };
        matches!(
            tail,
            Expr::Call(callee, args)
                if args.len() == 3
                    && matches!(
                        callee.as_ref(),
                        Expr::Member(obj, name)
                            if name == "bytes_str" && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")
                    )
        )
    }

    fn scan_expr(expr: &Expr, locals: &HashSet<String>, usage: &mut HashMap<String, LocalUsage>) {
        if let Some((method, arg)) = str_view_call_target(expr) {
            if locals.contains(arg) {
                let entry = usage.entry(arg.to_owned()).or_default();
                entry.reads += 1;
                if method == "str_bytes" {
                    entry.str_bytes_reads += 1;
                }
            }
            return;
        }
        match expr {
            Expr::Ident(name) => {
                if locals.contains(name) {
                    usage.entry(name.clone()).or_default().disqualified = true;
                }
            }
            Expr::Member(obj, _) => scan_expr(obj, locals, usage),
            Expr::Index(obj, idx) => {
                scan_expr(obj, locals, usage);
                scan_expr(idx, locals, usage);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                scan_expr(c, locals, usage);
                for a in args {
                    scan_expr(a, locals, usage);
                }
            }
            Expr::Unary(_, inner) => scan_expr(inner, locals, usage),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                scan_expr(l, locals, usage);
                scan_expr(r, locals, usage);
            }
            Expr::Cond(c, t, e) => {
                scan_expr(c, locals, usage);
                scan_expr(t, locals, usage);
                scan_expr(e, locals, usage);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    scan_expr(v, locals, usage);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    scan_expr(item, locals, usage);
                }
            }
            Expr::Arrow(params, ArrowBody::Expr(b)) => {
                let shadow: HashSet<String> = locals
                    .iter()
                    .filter(|n| !params.contains(*n))
                    .cloned()
                    .collect();
                // Capturing a local in an arrow disqualifies rewriting it because rewrites stay local.
                let mut inner_usage = HashMap::new();
                scan_expr(b, &shadow, &mut inner_usage);
                for (name, u) in inner_usage {
                    if u.writes > 0 || u.reads > 0 || u.disqualified {
                        usage.entry(name).or_default().disqualified = true;
                    }
                }
            }
            Expr::Arrow(params, ArrowBody::Block(stmts)) => {
                let shadow: HashSet<String> = locals
                    .iter()
                    .filter(|n| !params.contains(*n))
                    .cloned()
                    .collect();
                let mut inner_usage = HashMap::new();
                scan_stmts(stmts, &shadow, &mut inner_usage);
                for (name, u) in inner_usage {
                    if u.writes > 0 || u.reads > 0 || u.disqualified {
                        usage.entry(name).or_default().disqualified = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn scan_stmts(
        stmts: &[Stmt],
        locals: &HashSet<String>,
        usage: &mut HashMap<String, LocalUsage>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(target, value)) if matches!(target.as_ref(), Expr::Ident(n) if locals.contains(n)) =>
                {
                    let Expr::Ident(name) = target.as_ref() else {
                        unreachable!()
                    };
                    if peel_bytes_str(value) {
                        usage.entry(name.clone()).or_default().writes += 1;
                    } else {
                        usage.entry(name.clone()).or_default().disqualified = true;
                    }
                    scan_expr(value, locals, usage);
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => {
                    scan_expr(e, locals, usage);
                }
                Stmt::Let(name, Some(init)) if locals.contains(name) => {
                    if peel_bytes_str(init) {
                        usage.entry(name.clone()).or_default().writes += 1;
                    } else {
                        usage.entry(name.clone()).or_default().disqualified = true;
                    }
                    scan_expr(init, locals, usage);
                }
                Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => scan_expr(e, locals, usage),
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            if locals.contains(name) {
                                if peel_bytes_str(init) {
                                    usage.entry(name.clone()).or_default().writes += 1;
                                } else {
                                    usage.entry(name.clone()).or_default().disqualified = true;
                                }
                            }
                            scan_expr(init, locals, usage);
                        }
                    }
                }
                Stmt::If(test, a, b) => {
                    scan_expr(test, locals, usage);
                    scan_stmts(a, locals, usage);
                    if let Some(b) = b {
                        scan_stmts(b, locals, usage);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_expr(d, locals, usage);
                    for c in cases {
                        for t in &c.tests {
                            scan_expr(t, locals, usage);
                        }
                        scan_stmts(&c.body, locals, usage);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    scan_stmts(std::slice::from_ref(inner.as_ref()), locals, usage);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => scan_stmts(b, locals, usage),
                Stmt::While(t, b) => {
                    scan_expr(t, locals, usage);
                    scan_stmts(b, locals, usage);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    if locals.contains(binding) {
                        usage.entry(binding.clone()).or_default().disqualified = true;
                    }
                    scan_stmts(body, locals, usage);
                    scan_stmts(handler, locals, usage);
                }
                Stmt::FunctionDecl { params, body, .. } => {
                    let shadow: HashSet<String> = locals
                        .iter()
                        .filter(|n| !params.contains(*n))
                        .cloned()
                        .collect();
                    let mut inner_usage = HashMap::new();
                    scan_stmts(body, &shadow, &mut inner_usage);
                    for (name, u) in inner_usage {
                        if u.writes > 0 || u.reads > 0 || u.disqualified {
                            usage.entry(name).or_default().disqualified = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn rewrite_bytes_str_rhs(expr: &mut Expr) {
        let taken = std::mem::replace(expr, Expr::Undefined);
        let (mut pre, tail) = match taken {
            Expr::Seq(mut items) if !items.is_empty() => {
                let tail = items.pop().expect("non-empty seq");
                (items, tail)
            }
            other => (Vec::new(), other),
        };
        let slice = match super::ptr::bytes_str_args(tail) {
            Ok((b, o, l)) => ptr::fat_slice(b, o, l),
            Err(other) => other,
        };
        *expr = if pre.is_empty() {
            slice
        } else {
            pre.push(slice);
            jsast::seq(pre)
        };
    }

    fn rewrite_expr(expr: &mut Expr, eligible: &HashSet<String>) {
        match expr {
            Expr::Call(callee, args) if args.len() == 1 => {
                let method = match callee.as_ref() {
                    Expr::Member(obj, method) if matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt") => {
                        Some(method.as_str())
                    }
                    _ => None,
                };
                if let (Some(method), Expr::Ident(arg)) = (method, &args[0]) {
                    if eligible.contains(arg) {
                        if method == "str_bytes" {
                            *expr =
                                jsast::rt_call("byte_slice_ref", vec![Expr::Ident(arg.clone())]);
                            return;
                        }
                        if method == "str_len" {
                            *expr = ptr::slot_len(Expr::Ident(arg.clone()));
                            return;
                        }
                    }
                }
                rewrite_expr(callee, eligible);
                for a in args {
                    rewrite_expr(a, eligible);
                }
            }
            Expr::Member(obj, _) => rewrite_expr(obj, eligible),
            Expr::Index(obj, idx) => {
                rewrite_expr(obj, eligible);
                rewrite_expr(idx, eligible);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                rewrite_expr(c, eligible);
                for a in args {
                    rewrite_expr(a, eligible);
                }
            }
            Expr::Unary(_, inner) => rewrite_expr(inner, eligible),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                rewrite_expr(l, eligible);
                rewrite_expr(r, eligible);
            }
            Expr::Cond(c, t, e) => {
                rewrite_expr(c, eligible);
                rewrite_expr(t, eligible);
                rewrite_expr(e, eligible);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    rewrite_expr(v, eligible);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    rewrite_expr(item, eligible);
                }
            }
            _ => {}
        }
    }

    fn rewrite_stmts(stmts: &mut [Stmt], eligible: &HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(target, value)) if matches!(target.as_ref(), Expr::Ident(n) if eligible.contains(n)) =>
                {
                    rewrite_expr(value, eligible);
                    rewrite_bytes_str_rhs(value);
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => {
                    rewrite_expr(e, eligible);
                }
                Stmt::Let(name, Some(init)) if eligible.contains(name) => {
                    rewrite_expr(init, eligible);
                    rewrite_bytes_str_rhs(init);
                }
                Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => rewrite_expr(e, eligible),
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            rewrite_expr(init, eligible);
                            if eligible.contains(name) {
                                rewrite_bytes_str_rhs(init);
                            }
                        }
                    }
                }
                Stmt::If(test, a, b) => {
                    rewrite_expr(test, eligible);
                    rewrite_stmts(a, eligible);
                    if let Some(b) = b {
                        rewrite_stmts(b, eligible);
                    }
                }
                Stmt::Switch(d, cases) => {
                    rewrite_expr(d, eligible);
                    for c in cases {
                        for t in &mut c.tests {
                            rewrite_expr(t, eligible);
                        }
                        rewrite_stmts(&mut c.body, eligible);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    rewrite_stmts(std::slice::from_mut(inner.as_mut()), eligible);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => rewrite_stmts(b, eligible),
                Stmt::While(t, b) => {
                    rewrite_expr(t, eligible);
                    rewrite_stmts(b, eligible);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    rewrite_stmts(body, eligible);
                    rewrite_stmts(handler, eligible);
                }
                _ => {}
            }
        }
    }

    let mut usage = HashMap::new();
    scan_stmts(body, locals, &mut usage);
    let policy = LocalFusePolicy {
        peel_rhs: peel_bytes_str,
        read_call: str_view_call_target,
        allow_early_exit: true,
    };
    let eligible: HashSet<String> = usage
        .into_iter()
        .filter(|(name, u)| {
            !u.disqualified
                && u.writes == 1
                && u.reads > 0
                && write_dominates_and_safe(body, name, u.reads, u.str_bytes_reads, &policy)
                    == Some(true)
        })
        .map(|(name, _)| name)
        .collect();
    if !eligible.is_empty() {
        rewrite_stmts(body, &eligible);
    }
}

fn rt_unary_call<'a>(expr: &'a Expr, method: &str) -> Option<&'a Expr> {
    let Expr::Call(callee, args) = expr else {
        return None;
    };
    let [arg] = args.as_slice() else {
        return None;
    };
    let Expr::Member(obj, name) = callee.as_ref() else {
        return None;
    };
    (name == method && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")).then_some(arg)
}

/// Folds nested `__rt.str_bytes(__rt.bstr_in(x))` and `__rt.bstr_bytes(__rt.bstr_in(x))` into
/// `__rt.bstr_utf8_bytes(x)` under `js-str=bytes`.
pub(crate) fn fold_bstr_in_bytes_decl(decl: &mut Stmt) {
    match decl {
        Stmt::FunctionDecl { body, .. } | Stmt::Block(body) => fold_bstr_in_bytes_stmts(body),
        _ => {}
    }
}

pub(crate) fn fold_bstr_in_bytes_stmts(stmts: &mut [Stmt]) {
    fn visit_expr(expr: &mut Expr) {
        match expr {
            Expr::Member(obj, _) | Expr::Unary(_, obj) => visit_expr(obj),
            Expr::Index(obj, idx)
            | Expr::Binary(_, obj, idx)
            | Expr::Assign(obj, idx)
            | Expr::CompoundAssign(_, obj, idx) => {
                visit_expr(obj);
                visit_expr(idx);
            }
            Expr::Call(callee, args) | Expr::New(callee, args) => {
                visit_expr(callee);
                for arg in args {
                    visit_expr(arg);
                }
            }
            Expr::Cond(c, t, e) => {
                visit_expr(c);
                visit_expr(t);
                visit_expr(e);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    visit_expr(v);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    visit_expr(item);
                }
            }
            Expr::Arrow(_, ArrowBody::Expr(inner)) => visit_expr(inner),
            Expr::Arrow(_, ArrowBody::Block(stmts)) => fold_bstr_in_bytes_stmts(stmts),
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::Ident(_)
            | Expr::Raw(_) => {}
        }

        let is_bytes_call = rt_unary_call(expr, "str_bytes").is_some()
            || rt_unary_call(expr, "bstr_bytes").is_some();
        if !is_bytes_call {
            return;
        }
        let Expr::Call(_, args) = expr else {
            return;
        };
        let inner = std::mem::replace(&mut args[0], Expr::Undefined);
        match inner {
            Expr::Call(ref c, ref inner_args)
                if inner_args.len() == 1
                    && matches!(
                        c.as_ref(),
                        Expr::Member(obj, name)
                            if name == "bstr_in" && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")
                    ) =>
            {
                let mut inner_args = match inner {
                    Expr::Call(_, a) => a,
                    _ => unreachable!(),
                };
                *expr = jsast::rt_call("bstr_utf8_bytes", vec![inner_args.swap_remove(0)]);
            }
            Expr::Seq(mut items) => {
                if let Some(last) = items.pop() {
                    if let Some(arg) = rt_unary_call(&last, "bstr_in").cloned() {
                        items.push(jsast::rt_call("bstr_utf8_bytes", vec![arg]));
                        *expr = jsast::seq(items);
                        return;
                    }
                    items.push(last);
                }
                args[0] = Expr::Seq(items);
            }
            other => {
                args[0] = other;
            }
        }
    }

    for stmt in stmts {
        match stmt {
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => visit_expr(e),
            Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => visit_expr(e),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        visit_expr(e);
                    }
                }
            }
            Stmt::If(test, a, b) => {
                visit_expr(test);
                fold_bstr_in_bytes_stmts(a);
                if let Some(b) = b {
                    fold_bstr_in_bytes_stmts(b);
                }
            }
            Stmt::Switch(d, cases) => {
                visit_expr(d);
                for c in cases {
                    for t in &mut c.tests {
                        visit_expr(t);
                    }
                    fold_bstr_in_bytes_stmts(&mut c.body);
                }
            }
            Stmt::Labeled(_, inner) => {
                fold_bstr_in_bytes_stmts(std::slice::from_mut(inner.as_mut()));
            }
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::FunctionDecl { body: b, .. } => {
                fold_bstr_in_bytes_stmts(b);
            }
            Stmt::While(t, b) => {
                visit_expr(t);
                fold_bstr_in_bytes_stmts(b);
            }
            Stmt::TryCatch { body, handler, .. } => {
                fold_bstr_in_bytes_stmts(body);
                fold_bstr_in_bytes_stmts(handler);
            }
            _ => {}
        }
    }
}

/// Fuses single-write `x = __rt.bstr_in(host)` whose sole read in the body is `__rt.str_bytes(x)`
/// into `x = host; ... __rt.bstr_utf8_bytes(x)` only when the write dominates the read with no
/// intervening side effects, branches, or early exits so `__rt.bstr_in`'s `TypeError` check runs at
/// the same observable point via `__rt.bstr_utf8_bytes`.
fn fuse_local_bstr_in_bytes(body: &mut [Stmt], locals: &HashSet<String>) {
    #[derive(Default)]
    struct Usage {
        writes: usize,
        reads: usize,
        disqualified: bool,
    }

    fn peel_bstr_in(expr: &Expr) -> bool {
        seq_tail(expr).is_some_and(|tail| rt_unary_call(tail, "bstr_in").is_some())
    }

    fn scan_expr(expr: &Expr, locals: &HashSet<String>, usage: &mut HashMap<String, Usage>) {
        if let Some(Expr::Ident(arg)) = rt_unary_call(expr, "str_bytes") {
            if locals.contains(arg) {
                usage.entry(arg.clone()).or_default().reads += 1;
                return;
            }
        }
        match expr {
            Expr::Ident(name) => {
                if locals.contains(name) {
                    usage.entry(name.clone()).or_default().disqualified = true;
                }
            }
            Expr::Member(obj, _) | Expr::Unary(_, obj) => scan_expr(obj, locals, usage),
            Expr::Index(obj, idx)
            | Expr::Binary(_, obj, idx)
            | Expr::Assign(obj, idx)
            | Expr::CompoundAssign(_, obj, idx) => {
                scan_expr(obj, locals, usage);
                scan_expr(idx, locals, usage);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                scan_expr(c, locals, usage);
                for a in args {
                    scan_expr(a, locals, usage);
                }
            }
            Expr::Cond(c, t, e) => {
                scan_expr(c, locals, usage);
                scan_expr(t, locals, usage);
                scan_expr(e, locals, usage);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    scan_expr(v, locals, usage);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    scan_expr(item, locals, usage);
                }
            }
            Expr::Arrow(params, body_expr) => {
                let shadow: HashSet<String> = locals
                    .iter()
                    .filter(|n| !params.contains(*n))
                    .cloned()
                    .collect();
                let mut inner = HashMap::new();
                match body_expr {
                    ArrowBody::Expr(b) => scan_expr(b, &shadow, &mut inner),
                    ArrowBody::Block(stmts) => scan_stmts(stmts, &shadow, &mut inner),
                }
                for (name, u) in inner {
                    if u.writes > 0 || u.reads > 0 || u.disqualified {
                        usage.entry(name).or_default().disqualified = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn scan_stmts(stmts: &[Stmt], locals: &HashSet<String>, usage: &mut HashMap<String, Usage>) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if locals.contains(n)) =>
                {
                    let Expr::Ident(name) = lhs.as_ref() else {
                        unreachable!()
                    };
                    let entry = usage.entry(name.clone()).or_default();
                    if peel_bstr_in(rhs) {
                        entry.writes += 1;
                    } else {
                        entry.disqualified = true;
                    }
                    scan_expr(rhs, locals, usage);
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => {
                    scan_expr(e, locals, usage);
                }
                Stmt::Let(name, Some(init)) if locals.contains(name) => {
                    let entry = usage.entry(name.clone()).or_default();
                    if peel_bstr_in(init) {
                        entry.writes += 1;
                    } else {
                        entry.disqualified = true;
                    }
                    scan_expr(init, locals, usage);
                }
                Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => {
                    scan_expr(e, locals, usage);
                }
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            if locals.contains(name) {
                                let entry = usage.entry(name.clone()).or_default();
                                if peel_bstr_in(init) {
                                    entry.writes += 1;
                                } else {
                                    entry.disqualified = true;
                                }
                            }
                            scan_expr(init, locals, usage);
                        }
                    }
                }
                Stmt::If(test, a, b) => {
                    scan_expr(test, locals, usage);
                    scan_stmts(a, locals, usage);
                    if let Some(b) = b {
                        scan_stmts(b, locals, usage);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_expr(d, locals, usage);
                    for c in cases {
                        for t in &c.tests {
                            scan_expr(t, locals, usage);
                        }
                        scan_stmts(&c.body, locals, usage);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    scan_stmts(std::slice::from_ref(inner.as_ref()), locals, usage);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => scan_stmts(b, locals, usage),
                Stmt::While(t, b) => {
                    scan_expr(t, locals, usage);
                    scan_stmts(b, locals, usage);
                }
                Stmt::TryCatch {
                    body,
                    binding,
                    handler,
                } => {
                    if locals.contains(binding) {
                        usage.entry(binding.clone()).or_default().disqualified = true;
                    }
                    scan_stmts(body, locals, usage);
                    scan_stmts(handler, locals, usage);
                }
                Stmt::FunctionDecl { params, body, .. } => {
                    let shadow: HashSet<String> = locals
                        .iter()
                        .filter(|n| !params.contains(*n))
                        .cloned()
                        .collect();
                    let mut inner = HashMap::new();
                    scan_stmts(body, &shadow, &mut inner);
                    for (name, u) in inner {
                        if u.writes > 0 || u.reads > 0 || u.disqualified {
                            usage.entry(name).or_default().disqualified = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn unwrap_bstr_in_rhs(expr: &mut Expr) {
        let taken = std::mem::replace(expr, Expr::Undefined);
        let (mut pre, tail) = match taken {
            Expr::Seq(mut items) if !items.is_empty() => {
                let tail = items.pop().expect("non-empty seq");
                (items, tail)
            }
            other => (Vec::new(), other),
        };
        let unwrapped = match tail {
            Expr::Call(c, mut args)
                if args.len() == 1
                    && matches!(
                        c.as_ref(),
                        Expr::Member(obj, name)
                            if name == "bstr_in" && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")
                    ) =>
            {
                args.swap_remove(0)
            }
            other => other,
        };
        *expr = if pre.is_empty() {
            unwrapped
        } else {
            pre.push(unwrapped);
            jsast::seq(pre)
        };
    }

    fn rewrite_expr(expr: &mut Expr, eligible: &HashSet<String>) {
        if let Some(Expr::Ident(arg)) = rt_unary_call(expr, "str_bytes") {
            if eligible.contains(arg) {
                let arg = arg.clone();
                *expr = jsast::rt_call("bstr_utf8_bytes", vec![Expr::Ident(arg)]);
                return;
            }
        }
        match expr {
            Expr::Member(obj, _) | Expr::Unary(_, obj) => rewrite_expr(obj, eligible),
            Expr::Index(obj, idx)
            | Expr::Binary(_, obj, idx)
            | Expr::Assign(obj, idx)
            | Expr::CompoundAssign(_, obj, idx) => {
                rewrite_expr(obj, eligible);
                rewrite_expr(idx, eligible);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                rewrite_expr(c, eligible);
                for a in args {
                    rewrite_expr(a, eligible);
                }
            }
            Expr::Cond(c, t, e) => {
                rewrite_expr(c, eligible);
                rewrite_expr(t, eligible);
                rewrite_expr(e, eligible);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    rewrite_expr(v, eligible);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    rewrite_expr(item, eligible);
                }
            }
            _ => {}
        }
    }

    fn rewrite_stmts(stmts: &mut [Stmt], eligible: &HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if eligible.contains(n)) =>
                {
                    unwrap_bstr_in_rhs(rhs);
                    rewrite_expr(rhs, eligible);
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => {
                    rewrite_expr(e, eligible);
                }
                Stmt::Let(name, Some(init)) => {
                    if eligible.contains(name) {
                        unwrap_bstr_in_rhs(init);
                    }
                    rewrite_expr(init, eligible);
                }
                Stmt::Return(Some(e)) => rewrite_expr(e, eligible),
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            if eligible.contains(name) {
                                unwrap_bstr_in_rhs(init);
                            }
                            rewrite_expr(init, eligible);
                        }
                    }
                }
                Stmt::If(test, a, b) => {
                    rewrite_expr(test, eligible);
                    rewrite_stmts(a, eligible);
                    if let Some(b) = b {
                        rewrite_stmts(b, eligible);
                    }
                }
                Stmt::Switch(d, cases) => {
                    rewrite_expr(d, eligible);
                    for c in cases {
                        for t in &mut c.tests {
                            rewrite_expr(t, eligible);
                        }
                        rewrite_stmts(&mut c.body, eligible);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    rewrite_stmts(std::slice::from_mut(inner.as_mut()), eligible);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => rewrite_stmts(b, eligible),
                Stmt::While(t, b) => {
                    rewrite_expr(t, eligible);
                    rewrite_stmts(b, eligible);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    rewrite_stmts(body, eligible);
                    rewrite_stmts(handler, eligible);
                }
                _ => {}
            }
        }
    }

    let mut usage = HashMap::new();
    scan_stmts(body, locals, &mut usage);
    let policy = LocalFusePolicy {
        peel_rhs: peel_bstr_in,
        read_call: str_bytes_call_target,
        allow_early_exit: false,
    };
    let eligible: HashSet<String> = usage
        .into_iter()
        .filter(|(name, u)| {
            !u.disqualified
                && u.writes == 1
                && u.reads == 1
                && write_dominates_and_safe(body, name, u.reads, u.reads, &policy) == Some(true)
        })
        .map(|(name, _)| name)
        .collect();
    if !eligible.is_empty() {
        rewrite_stmts(body, &eligible);
    }
}

/// Removes assignments to compiler locals that no expression ever reads. JIR values are SSA,
/// but range/projection cleanup can leave a pure snapshot behind after the last JIR DCE run. At
/// this point the structured JavaScript is authoritative, so a function-wide zero-read test is
/// both smaller and more exact than trying to predict the structurizer's final spelling.
fn eliminate_dead_local_writes(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
    loop {
        let mut mentions: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for stmt in body.iter() {
            jsast::visit_idents(stmt, &mut |name| {
                *mentions.entry(name.to_owned()).or_default() += 1;
            });
        }
        let mut writes: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        count_simple_writes(body, &mut writes);
        let unread: HashSet<String> = writes
            .iter()
            .filter(|(name, count)| {
                locals.contains(*name) && mentions.get(*name).copied().unwrap_or(0) == **count
            })
            .map(|(name, _)| name.clone())
            .collect();
        if unread.is_empty() || !remove_unread_writes(body, &unread) {
            return;
        }
    }
}

fn count_simple_writes(body: &[Stmt], writes: &mut std::collections::HashMap<String, usize>) {
    for stmt in body {
        match stmt {
            Stmt::ExprStmt(jsast::Expr::Assign(target, _)) => {
                if let jsast::Expr::Ident(name) = target.as_ref() {
                    *writes.entry(name.clone()).or_default() += 1;
                }
            }
            Stmt::If(_, then, els) => {
                count_simple_writes(then, writes);
                if let Some(els) = els {
                    count_simple_writes(els, writes);
                }
            }
            Stmt::Switch(_, cases) => {
                for case in cases {
                    count_simple_writes(&case.body, writes);
                }
            }
            Stmt::Labeled(_, inner) => count_simple_writes(std::slice::from_ref(inner), writes),
            Stmt::LoopForever(_, inner) | Stmt::While(_, inner) | Stmt::Block(inner) => {
                count_simple_writes(inner, writes);
            }
            _ => {}
        }
    }
}

fn remove_unread_writes(body: &mut Vec<Stmt>, unread: &HashSet<String>) -> bool {
    let mut changed = false;
    let mut kept = Vec::with_capacity(body.len());
    for mut stmt in std::mem::take(body) {
        match &mut stmt {
            Stmt::If(_, then, els) => {
                changed |= remove_unread_writes(then, unread);
                if let Some(els) = els {
                    changed |= remove_unread_writes(els, unread);
                }
            }
            Stmt::Switch(_, cases) => {
                for case in cases {
                    changed |= remove_unread_writes(&mut case.body, unread);
                }
            }
            Stmt::Labeled(_, inner) => {
                if let Stmt::Block(body) = inner.as_mut() {
                    changed |= remove_unread_writes(body, unread);
                }
            }
            Stmt::LoopForever(_, inner) | Stmt::While(_, inner) | Stmt::Block(inner) => {
                changed |= remove_unread_writes(inner, unread);
            }
            _ => {}
        }
        let replacement = match stmt {
            Stmt::ExprStmt(jsast::Expr::Assign(target, value)) if matches!(target.as_ref(), jsast::Expr::Ident(name) if unread.contains(name)) =>
            {
                changed = true;
                (!jsast::is_droppable(&value)).then_some(Stmt::ExprStmt(*value))
            }
            other => Some(other),
        };
        if let Some(stmt) = replacement {
            kept.push(stmt);
        }
    }
    *body = kept;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_local_writes_are_removed_without_dropping_effects() {
        let mut body = vec![
            jsast::assign_stmt(jsast::id("dead"), jsast::object(Vec::new())),
            jsast::assign_stmt(
                jsast::id("effect"),
                jsast::call(jsast::id("host"), Vec::new()),
            ),
            jsast::assign_stmt(jsast::id("live"), jsast::num(3.0)),
            jsast::assign_stmt(jsast::id("external"), jsast::num(4.0)),
            Stmt::If(
                jsast::id("condition"),
                vec![jsast::assign_stmt(
                    jsast::id("nested_dead"),
                    jsast::array(Vec::new()),
                )],
                None,
            ),
            Stmt::Return(Some(jsast::id("live"))),
        ];
        let locals = ["dead", "effect", "live", "nested_dead"]
            .into_iter()
            .map(str::to_owned)
            .collect();

        eliminate_dead_local_writes(&mut body, &locals);

        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("dead ="), "{printed}");
        assert!(!printed.contains("effect ="), "{printed}");
        assert!(!printed.contains("nested_dead ="), "{printed}");
        assert!(printed.contains("host();"), "{printed}");
        assert!(printed.contains("live = 3;"), "{printed}");
        assert!(printed.contains("external = 4;"), "{printed}");
        assert!(printed.contains("return live;"), "{printed}");
    }

    #[test]
    fn dead_local_cleanup_reaches_a_fixed_point() {
        let mut body = vec![
            jsast::assign_stmt(jsast::id("inner"), jsast::object(Vec::new())),
            jsast::assign_stmt(jsast::id("outer"), jsast::id("inner")),
        ];
        let locals = ["inner", "outer"].into_iter().map(str::to_owned).collect();

        eliminate_dead_local_writes(&mut body, &locals);

        assert!(
            body.is_empty(),
            "{}",
            jsast::stmt_to_string(&Stmt::Block(body))
        );
    }

    #[test]
    fn fuse_local_bstr_in_bytes_respects_dominance_and_purity() {
        let rt_call = |method: &str, arg: Expr| jsast::rt_call(method, vec![arg]);
        let locals: HashSet<String> = ["x".to_owned(), "y".to_owned()].into_iter().collect();

        // Pure straight-line sequence fuses cleanly.
        let mut pure_body = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            jsast::assign_stmt(jsast::id("y"), jsast::num(1.0)),
            Stmt::Return(Some(rt_call("str_bytes", jsast::id("x")))),
        ];
        fuse_local_bstr_in_bytes(&mut pure_body, &locals);
        let printed_pure = jsast::stmt_to_string(&Stmt::Block(pure_body));
        assert!(printed_pure.contains("x = host;"), "{printed_pure}");
        assert!(
            printed_pure.contains("__rt.bstr_utf8_bytes(x)"),
            "{printed_pure}"
        );
        assert!(!printed_pure.contains("bstr_in"), "{printed_pure}");

        // Intervening impure call blocks fusion so `bstr_in`'s TypeError runs first.
        let mut impure_body = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::ExprStmt(jsast::call(jsast::id("host_log"), Vec::new())),
            Stmt::Return(Some(rt_call("str_bytes", jsast::id("x")))),
        ];
        fuse_local_bstr_in_bytes(&mut impure_body, &locals);
        let printed_impure = jsast::stmt_to_string(&Stmt::Block(impure_body));
        assert!(printed_impure.contains("bstr_in"), "{printed_impure}");
        assert!(
            !printed_impure.contains("bstr_utf8_bytes"),
            "{printed_impure}"
        );

        // Early return / conditional read / short-circuit logical op blocks fusion.
        let mut early_ret = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::If(jsast::id("flag"), vec![Stmt::Return(None)], None),
            Stmt::Return(Some(rt_call("str_bytes", jsast::id("x")))),
        ];
        fuse_local_bstr_in_bytes(&mut early_ret, &locals);
        assert!(
            jsast::stmt_to_string(&Stmt::Block(early_ret)).contains("bstr_in"),
            "early return must block fusion"
        );

        let mut short_circuit = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::Return(Some(Expr::Binary(
                jsast::BinOp::And,
                Box::new(jsast::id("flag")),
                Box::new(rt_call("str_bytes", jsast::id("x"))),
            ))),
        ];
        fuse_local_bstr_in_bytes(&mut short_circuit, &locals);
        assert!(
            jsast::stmt_to_string(&Stmt::Block(short_circuit)).contains("bstr_in"),
            "short-circuit And must block fusion"
        );

        let mut cond_arm = vec![
            jsast::assign_stmt(jsast::id("x"), rt_call("bstr_in", jsast::id("host"))),
            Stmt::Return(Some(Expr::Cond(
                Box::new(jsast::id("flag")),
                Box::new(rt_call("str_bytes", jsast::id("x"))),
                Box::new(jsast::num(0.0)),
            ))),
        ];
        fuse_local_bstr_in_bytes(&mut cond_arm, &locals);
        assert!(
            jsast::stmt_to_string(&Stmt::Block(cond_arm)).contains("bstr_in"),
            "conditional arm must block fusion"
        );
    }
}
