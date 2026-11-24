//! One item as a top level declaration: a function (`base.rs::codegen_fn` and `codegen_body`),
//! a `static` (`base.rs::codegen_static`), or a vtable (`vtable.rs`).

use std::collections::{BTreeSet, HashMap, HashSet};

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::{Decline, OpClass};
use rcgjs_object::jir::repr::Func;
use rcgjs_object::jir::semantic::{BodyKind, BodySemantic, VtableEntry};
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
    if input.heap.is_some_and(|h| h.native_string_enabled) {
        if let BodySemantic::DropGlue(ty) = input.body.semantic {
            if input.table.get(ty).is_some_and(|d| {
                d.meta.candidate == rcgjs_object::jir::types::RepresentationCandidate::String
            }) {
                let params: Vec<String> =
                    (0..func.params.len()).map(|i| format!("$p{i}")).collect();
                let decl = jsast::function(input.name.as_str(), params, Vec::new());
                return Ok(LoweredItem {
                    decl,
                    refs: BTreeSet::new(),
                    zombies: Vec::new(),
                    loc: None,
                    names: Vec::new(),
                    dts: None,
                    js_alloc_origins: Vec::new(),
                    bigints: BTreeSet::new(),
                });
            }
        }
    }
    if input.body.semantic
        == BodySemantic::Standard(rcgjs_object::jir::semantic::StandardOperation::SliceSort)
    {
        if let Some(lowered) = try_lower_slice_sort(input, func, physical)? {
            return Ok(lowered);
        }
    }
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
    .with_heap(input.heap)
    .with_nested_clone(input.nested_clone)
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
        eliminate_dead_local_writes(&mut body, &local_names);
        fuse_local_bytes_str_with(&mut body, &uninit_locals, cx.needs_byte_slice_ref());
        fold_offset_slot_derefs(&mut body, &uninit_locals);
        eliminate_dead_local_writes(&mut body, &local_names);
        fold_sfp_deref_only(&mut body, &uninit_locals);
        if crate::lower::bstr::enabled() {
            fuse_local_bstr_in_bytes(&mut body, &uninit_locals);
        }
        if input.opts.profile == Some(crate::opts::Profile::ReleaseSpeed)
            && !cx.byte_views.raw_byte_derefs
            && cx.byte_views.elide_scale_scalar
        {
            cse_slice_loads(&mut body, &mut declarations, &mut local_names);
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

    if input.body.semantic == BodySemantic::PanicImpl {
        out = jsast::wrap_panic_impl_body(out);
    }

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

/// Host-delegated lowering for `core::slice::sort::{stable,unstable}::sort`
/// (`StandardOperation::SliceSort`). Replaces the fallback insertion-sort body with a direct call
/// to `__rt.slice_sort(v.buf, v.off, v.len, is_less, cmp_fn, is_indirect)` (or a bare `return;`
/// for ZST element types), keeping `cmp_fn` as the sole item reference.
fn try_lower_slice_sort(
    input: &LowerInput<'_>,
    func: &Func,
    physical: Option<&crate::plan::abi_flat::FlatSig>,
) -> Result<Option<LoweredItem>, Decline> {
    use rcgjs_object::jir::repr::OpKind;
    use rcgjs_object::jir::semantic::{CallSemantic, Callee, StandardOperation};
    use rcgjs_object::jir::types::TypeKind;

    if physical.is_some()
        || func.params.len() != 2
        || func.sig.abi.caller_location
        || func.sig.abi.spread.is_some()
    {
        return Ok(None);
    }
    let Some(&slice_ref_ty) = func.sig.arguments.first() else {
        return Ok(None);
    };
    let Some(TypeKind::Reference {
        pointee,
        mutable: true,
    }) = input.table.get(slice_ref_ty).map(|d| &d.kind)
    else {
        return Ok(None);
    };
    let Some(TypeKind::Slice(elem_ty)) = input.table.get(*pointee).map(|d| &d.kind) else {
        return Ok(None);
    };
    let Some(elem_desc) = input.table.get(*elem_ty) else {
        return Ok(None);
    };
    if !elem_desc.layout.sized {
        return Ok(None);
    }
    let params = vec!["_v".to_owned(), "_is_less".to_owned()];
    if elem_desc.layout.zst {
        let decl = jsast::function(input.name.as_str(), params, vec![jsast::ret_void()]);
        return finish(
            input,
            decl,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            input.bigints,
        )
        .map(Some);
    }
    let mut cmp_target = None;
    'search: for block in &func.blocks {
        for op in &block.ops {
            if let OpKind::Call {
                callee: Callee::Direct { item, link_name },
                semantic: CallSemantic::Standard(StandardOperation::SliceSortCmp),
                ..
            } = &op.kind
            {
                cmp_target = Some((*item, *link_name));
                break 'search;
            }
        }
    }
    let Some((cmp_item, cmp_link_name)) = cmp_target else {
        return Ok(None);
    };
    let cmp_name = match input.call_retargets.get(&cmp_item) {
        Some(name) => name.as_str().to_owned(),
        None => {
            let Some(name) = input.interns.str(cmp_link_name) else {
                return Ok(None);
            };
            name.to_owned()
        }
    };
    let is_indirect = input.table.is_indirect(*elem_ty);
    let call = jsast::rt_call(
        "slice_sort",
        vec![
            jsast::member(jsast::id("_v"), "buf"),
            jsast::member(jsast::id("_v"), "off"),
            jsast::member(jsast::id("_v"), "len"),
            jsast::id("_is_less"),
            jsast::id(cmp_name),
            jsast::boolean(is_indirect),
        ],
    );
    let decl = jsast::function(input.name.as_str(), params, vec![jsast::expr_stmt(call)]);
    finish(
        input,
        decl,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        input.bigints,
    )
    .map(Some)
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
#[cfg(test)]
pub(super) fn fuse_local_bytes_str(body: &mut [Stmt], locals: &HashSet<String>) {
    fuse_local_bytes_str_with(body, locals, true);
}

pub(super) fn fuse_local_bytes_str_with(
    body: &mut [Stmt],
    locals: &HashSet<String>,
    wrap_byte_slice_ref: bool,
) {
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

    fn rewrite_expr(expr: &mut Expr, eligible: &HashSet<String>, wrap_byte_slice_ref: bool) {
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
                            *expr = if wrap_byte_slice_ref {
                                jsast::rt_call("byte_slice_ref", vec![Expr::Ident(arg.clone())])
                            } else {
                                Expr::Ident(arg.clone())
                            };
                            return;
                        }
                        if method == "str_len" {
                            *expr = ptr::slot_len(Expr::Ident(arg.clone()));
                            return;
                        }
                    }
                }
                rewrite_expr(callee, eligible, wrap_byte_slice_ref);
                for a in args {
                    rewrite_expr(a, eligible, wrap_byte_slice_ref);
                }
            }
            Expr::Member(obj, _) => rewrite_expr(obj, eligible, wrap_byte_slice_ref),
            Expr::Index(obj, idx) => {
                rewrite_expr(obj, eligible, wrap_byte_slice_ref);
                rewrite_expr(idx, eligible, wrap_byte_slice_ref);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                rewrite_expr(c, eligible, wrap_byte_slice_ref);
                for a in args {
                    rewrite_expr(a, eligible, wrap_byte_slice_ref);
                }
            }
            Expr::Unary(_, inner) => rewrite_expr(inner, eligible, wrap_byte_slice_ref),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                rewrite_expr(l, eligible, wrap_byte_slice_ref);
                rewrite_expr(r, eligible, wrap_byte_slice_ref);
            }
            Expr::Cond(c, t, e) => {
                rewrite_expr(c, eligible, wrap_byte_slice_ref);
                rewrite_expr(t, eligible, wrap_byte_slice_ref);
                rewrite_expr(e, eligible, wrap_byte_slice_ref);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    rewrite_expr(v, eligible, wrap_byte_slice_ref);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    rewrite_expr(item, eligible, wrap_byte_slice_ref);
                }
            }
            _ => {}
        }
    }

    fn rewrite_stmts(stmts: &mut [Stmt], eligible: &HashSet<String>, wrap_byte_slice_ref: bool) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(target, value)) if matches!(target.as_ref(), Expr::Ident(n) if eligible.contains(n)) =>
                {
                    rewrite_expr(value, eligible, wrap_byte_slice_ref);
                    rewrite_bytes_str_rhs(value);
                }
                Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Const(_, e) => {
                    rewrite_expr(e, eligible, wrap_byte_slice_ref);
                }
                Stmt::Let(name, Some(init)) if eligible.contains(name) => {
                    rewrite_expr(init, eligible, wrap_byte_slice_ref);
                    rewrite_bytes_str_rhs(init);
                }
                Stmt::Let(_, Some(e)) | Stmt::Return(Some(e)) => {
                    rewrite_expr(e, eligible, wrap_byte_slice_ref)
                }
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            rewrite_expr(init, eligible, wrap_byte_slice_ref);
                            if eligible.contains(name) {
                                rewrite_bytes_str_rhs(init);
                            }
                        }
                    }
                }
                Stmt::If(test, a, b) => {
                    rewrite_expr(test, eligible, wrap_byte_slice_ref);
                    rewrite_stmts(a, eligible, wrap_byte_slice_ref);
                    if let Some(b) = b {
                        rewrite_stmts(b, eligible, wrap_byte_slice_ref);
                    }
                }
                Stmt::Switch(d, cases) => {
                    rewrite_expr(d, eligible, wrap_byte_slice_ref);
                    for c in cases {
                        for t in &mut c.tests {
                            rewrite_expr(t, eligible, wrap_byte_slice_ref);
                        }
                        rewrite_stmts(&mut c.body, eligible, wrap_byte_slice_ref);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    rewrite_stmts(
                        std::slice::from_mut(inner.as_mut()),
                        eligible,
                        wrap_byte_slice_ref,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    rewrite_stmts(b, eligible, wrap_byte_slice_ref)
                }
                Stmt::While(t, b) => {
                    rewrite_expr(t, eligible, wrap_byte_slice_ref);
                    rewrite_stmts(b, eligible, wrap_byte_slice_ref);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    rewrite_stmts(body, eligible, wrap_byte_slice_ref);
                    rewrite_stmts(handler, eligible, wrap_byte_slice_ref);
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
        rewrite_stmts(body, &eligible, wrap_byte_slice_ref);
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
    fn count_expr_writes(expr: &Expr, writes: &mut std::collections::HashMap<String, usize>) {
        match expr {
            Expr::Assign(target, _) => {
                if let Expr::Ident(name) = target.as_ref() {
                    *writes.entry(name.clone()).or_default() += 1;
                }
            }
            Expr::Seq(items) => {
                for item in items {
                    count_expr_writes(item, writes);
                }
            }
            _ => {}
        }
    }
    for stmt in body {
        match stmt {
            Stmt::ExprStmt(expr) => count_expr_writes(expr, writes),
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
    fn strip_expr(expr: Expr, unread: &HashSet<String>, changed: &mut bool) -> Option<Expr> {
        match expr {
            Expr::Assign(target, value) if matches!(target.as_ref(), Expr::Ident(name) if unread.contains(name)) =>
            {
                *changed = true;
                (!jsast::is_droppable(&value)).then_some(*value)
            }
            Expr::Seq(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if let Some(kept) = strip_expr(item, unread, changed) {
                        out.push(kept);
                    }
                }
                match out.len() {
                    0 => None,
                    1 => out.pop(),
                    _ => Some(Expr::Seq(out)),
                }
            }
            other => Some(other),
        }
    }
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
            Stmt::ExprStmt(expr) => strip_expr(expr, unread, &mut changed).map(Stmt::ExprStmt),
            other => Some(other),
        };
        if let Some(stmt) = replacement {
            kept.push(stmt);
        }
    }
    *body = kept;
    changed
}

/// Folds single-assignment local slot objects (`v = idx === 0 ? base : { buf: base.buf, off:
/// base.off + idx }`, `v = { buf: base.buf, off: base.off + idx }`, or `v = { buf: arr, off: k }`)
/// directly into `base.buf[base.off + idx]` / `arr[k]` when every read of `v` in the function is
/// strictly `v.buf[v.off]` within the same block and no dependency is mutated before the last use.
///
/// Eliminates per-iteration `{ buf, off }` slot allocations in `slice::Iter`, `slice::IterMut`,
/// `vec::IntoIter`, and `slice::swap` even across `Invoke` block boundaries where `fuse.rs` keeps
/// the offset result materialized.
pub(super) fn fold_offset_slot_derefs(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
    #[derive(Default)]
    struct SlotUsage {
        writes: usize,
        slot_derefs: usize,
        other_uses: usize,
    }

    #[derive(Clone)]
    enum SimpleIdx {
        Ident(String),
        Num(f64),
    }

    impl SimpleIdx {
        fn to_expr(&self) -> Expr {
            match self {
                Self::Ident(s) => Expr::Ident(s.clone()),
                Self::Num(n) => Expr::Num(*n),
            }
        }
        fn eq_idx(&self, other: &Self) -> bool {
            match (self, other) {
                (Self::Ident(a), Self::Ident(b)) => a == b,
                (Self::Num(a), Self::Num(b)) => a.to_bits() == b.to_bits(),
                _ => false,
            }
        }
    }

    #[derive(Clone)]
    enum SlotDef {
        Offset { base: String, idx: SimpleIdx },
        Direct { buf: String, off: SimpleIdx },
    }

    impl SlotDef {
        fn replacement(&self) -> Expr {
            match self {
                Self::Offset { base, idx } => jsast::index(
                    jsast::member(Expr::Ident(base.clone()), "buf"),
                    jsast::binary(
                        jsast::BinOp::Add,
                        jsast::member(Expr::Ident(base.clone()), "off"),
                        idx.to_expr(),
                    ),
                ),
                Self::Direct { buf, off } => jsast::index(Expr::Ident(buf.clone()), off.to_expr()),
            }
        }

        fn deps(&self) -> Vec<&str> {
            let mut out = Vec::with_capacity(2);
            match self {
                Self::Offset { base, idx } => {
                    out.push(base.as_str());
                    if let SimpleIdx::Ident(i) = idx {
                        out.push(i.as_str());
                    }
                }
                Self::Direct { buf, off } => {
                    out.push(buf.as_str());
                    if let SimpleIdx::Ident(i) = off {
                        out.push(i.as_str());
                    }
                }
            }
            out
        }

        fn is_offset(&self) -> bool {
            matches!(self, Self::Offset { .. })
        }
    }

    fn match_simple_idx(expr: &Expr) -> Option<SimpleIdx> {
        match expr {
            Expr::Ident(s) => Some(SimpleIdx::Ident(s.clone())),
            Expr::Num(n) => Some(SimpleIdx::Num(*n)),
            _ => None,
        }
    }

    fn match_slot_deref(expr: &Expr) -> Option<&str> {
        let Expr::Index(obj, idx) = expr else {
            return None;
        };
        let (Expr::Member(b_obj, b_prop), Expr::Member(o_obj, o_prop)) =
            (obj.as_ref(), idx.as_ref())
        else {
            return None;
        };
        if b_prop != "buf" || o_prop != "off" {
            return None;
        }
        match (b_obj.as_ref(), o_obj.as_ref()) {
            (Expr::Ident(a), Expr::Ident(b)) if a == b => Some(a.as_str()),
            _ => None,
        }
    }

    fn match_offset_obj(expr: &Expr) -> Option<(String, SimpleIdx)> {
        let Expr::Object(entries) = expr else {
            return None;
        };
        if entries.len() != 2 || entries[0].0 != "buf" || entries[1].0 != "off" {
            return None;
        }
        let Expr::Member(b1, bp) = &entries[0].1 else {
            return None;
        };
        let Expr::Ident(base1) = b1.as_ref() else {
            return None;
        };
        if bp != "buf" {
            return None;
        }
        let Expr::Binary(jsast::BinOp::Add, lhs, rhs) = &entries[1].1 else {
            return None;
        };
        let Expr::Member(b2, op) = lhs.as_ref() else {
            return None;
        };
        let Expr::Ident(base2) = b2.as_ref() else {
            return None;
        };
        if op != "off" || base1 != base2 {
            return None;
        }
        let idx = match_simple_idx(rhs)?;
        Some((base1.clone(), idx))
    }

    fn match_slot_def(target: &str, rhs: &Expr) -> Option<SlotDef> {
        // Case 1: `idx === 0 ? base : { buf: base.buf, off: base.off + idx }`
        if let Expr::Cond(cond, then_e, else_e) = rhs {
            if let (
                Expr::Binary(jsast::BinOp::StrictEq, c_lhs, c_rhs),
                Expr::Ident(then_base),
                Some((else_base, else_idx)),
            ) = (cond.as_ref(), then_e.as_ref(), match_offset_obj(else_e))
            {
                let c_zero = matches!(c_rhs.as_ref(), Expr::Num(n) if *n == 0.0);
                if c_zero
                    && then_base == &else_base
                    && match_simple_idx(c_lhs).is_some_and(|ci| ci.eq_idx(&else_idx))
                {
                    let def = SlotDef::Offset {
                        base: else_base,
                        idx: else_idx,
                    };
                    if !def.deps().contains(&target) {
                        return Some(def);
                    }
                }
            }
            return None;
        }
        // Case 2: `{ buf: base.buf, off: base.off + idx }`
        if let Some((base, idx)) = match_offset_obj(rhs) {
            let def = SlotDef::Offset { base, idx };
            if !def.deps().contains(&target) {
                return Some(def);
            }
            return None;
        }
        // Case 3: `{ buf: arr, off: k }`
        if let Expr::Object(entries) = rhs {
            if entries.len() == 2 && entries[0].0 == "buf" && entries[1].0 == "off" {
                if let (Expr::Ident(buf), Some(off)) =
                    (&entries[0].1, match_simple_idx(&entries[1].1))
                {
                    let def = SlotDef::Direct {
                        buf: buf.clone(),
                        off,
                    };
                    if !def.deps().contains(&target) {
                        return Some(def);
                    }
                }
            }
        }
        None
    }

    fn scan_expr(
        expr: &Expr,
        locals: &HashSet<String>,
        in_closure: bool,
        usage: &mut HashMap<String, SlotUsage>,
    ) {
        if !in_closure {
            if let Some(name) = match_slot_deref(expr) {
                if locals.contains(name) {
                    usage.entry(name.to_owned()).or_default().slot_derefs += 1;
                    return;
                }
            }
        }
        match expr {
            Expr::Ident(name) => {
                if locals.contains(name) {
                    usage.entry(name.clone()).or_default().other_uses += 1;
                }
            }
            Expr::Assign(lhs, rhs) => {
                if !in_closure {
                    if let Some(name) = match_slot_deref(lhs) {
                        if locals.contains(name) {
                            usage.entry(name.to_owned()).or_default().slot_derefs += 1;
                            scan_expr(rhs, locals, in_closure, usage);
                            return;
                        }
                    }
                }
                scan_expr(lhs, locals, in_closure, usage);
                scan_expr(rhs, locals, in_closure, usage);
            }
            Expr::CompoundAssign(_, lhs, rhs) => {
                scan_expr(lhs, locals, in_closure, usage);
                scan_expr(rhs, locals, in_closure, usage);
            }
            Expr::Member(obj, _) => scan_expr(obj, locals, in_closure, usage),
            Expr::Index(obj, idx) => {
                scan_expr(obj, locals, in_closure, usage);
                scan_expr(idx, locals, in_closure, usage);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                scan_expr(c, locals, in_closure, usage);
                for a in args {
                    scan_expr(a, locals, in_closure, usage);
                }
            }
            Expr::Unary(_, inner) => scan_expr(inner, locals, in_closure, usage),
            Expr::Binary(_, l, r) => {
                scan_expr(l, locals, in_closure, usage);
                scan_expr(r, locals, in_closure, usage);
            }
            Expr::Cond(c, t, e) => {
                scan_expr(c, locals, in_closure, usage);
                scan_expr(t, locals, in_closure, usage);
                scan_expr(e, locals, in_closure, usage);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    scan_expr(v, locals, in_closure, usage);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    scan_expr(x, locals, in_closure, usage);
                }
            }
            Expr::Arrow(_, jsast::ArrowBody::Expr(b)) => scan_expr(b, locals, true, usage),
            Expr::Arrow(_, jsast::ArrowBody::Block(stmts)) => {
                scan_stmts(stmts, locals, true, usage)
            }
            _ => {}
        }
    }

    fn scan_stmts(
        stmts: &[Stmt],
        locals: &HashSet<String>,
        in_closure: bool,
        usage: &mut HashMap<String, SlotUsage>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                    if !in_closure
                        && matches!(lhs.as_ref(), Expr::Ident(n) if locals.contains(n)) =>
                {
                    let Expr::Ident(name) = lhs.as_ref() else {
                        unreachable!()
                    };
                    usage.entry(name.clone()).or_default().writes += 1;
                    scan_expr(rhs, locals, in_closure, usage);
                }
                Stmt::Let(name, Some(init)) if !in_closure && locals.contains(name) => {
                    usage.entry(name.clone()).or_default().writes += 1;
                    scan_expr(init, locals, in_closure, usage);
                }
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => scan_expr(e, locals, in_closure, usage),
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            if !in_closure && locals.contains(name) {
                                usage.entry(name.clone()).or_default().writes += 1;
                            }
                            scan_expr(init, locals, in_closure, usage);
                        }
                    }
                }
                Stmt::If(t, a, b) => {
                    scan_expr(t, locals, in_closure, usage);
                    scan_stmts(a, locals, in_closure, usage);
                    if let Some(b) = b {
                        scan_stmts(b, locals, in_closure, usage);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_expr(d, locals, in_closure, usage);
                    for c in cases {
                        for t in &c.tests {
                            scan_expr(t, locals, in_closure, usage);
                        }
                        scan_stmts(&c.body, locals, in_closure, usage);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    scan_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        locals,
                        in_closure,
                        usage,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    scan_stmts(b, locals, in_closure, usage);
                }
                Stmt::FunctionDecl { body: b, .. } => scan_stmts(b, locals, true, usage),
                Stmt::While(t, b) => {
                    scan_expr(t, locals, in_closure, usage);
                    scan_stmts(b, locals, in_closure, usage);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    scan_stmts(body, locals, in_closure, usage);
                    scan_stmts(handler, locals, in_closure, usage);
                }
                _ => {}
            }
        }
    }

    fn count_derefs_in_expr(expr: &Expr, target: &str) -> usize {
        if match_slot_deref(expr) == Some(target) {
            return 1;
        }
        match expr {
            Expr::Member(obj, _) => count_derefs_in_expr(obj, target),
            Expr::Index(obj, idx) => {
                count_derefs_in_expr(obj, target) + count_derefs_in_expr(idx, target)
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                count_derefs_in_expr(c, target)
                    + args
                        .iter()
                        .map(|a| count_derefs_in_expr(a, target))
                        .sum::<usize>()
            }
            Expr::Unary(_, inner) => count_derefs_in_expr(inner, target),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                count_derefs_in_expr(l, target) + count_derefs_in_expr(r, target)
            }
            Expr::Cond(c, t, e) => {
                count_derefs_in_expr(c, target)
                    + count_derefs_in_expr(t, target)
                    + count_derefs_in_expr(e, target)
            }
            Expr::Object(entries) => entries
                .iter()
                .map(|(_, v)| count_derefs_in_expr(v, target))
                .sum(),
            Expr::Array(items) | Expr::Seq(items) => {
                items.iter().map(|x| count_derefs_in_expr(x, target)).sum()
            }
            _ => 0,
        }
    }

    fn count_derefs_in_stmt(stmt: &Stmt, target: &str) -> usize {
        match stmt {
            Stmt::ExprStmt(e)
            | Stmt::Throw(e)
            | Stmt::Const(_, e)
            | Stmt::Let(_, Some(e))
            | Stmt::Return(Some(e)) => count_derefs_in_expr(e, target),
            Stmt::Lets(decls) => decls
                .iter()
                .filter_map(|(_, init)| init.as_ref())
                .map(|e| count_derefs_in_expr(e, target))
                .sum(),
            Stmt::If(t, a, b) => {
                count_derefs_in_expr(t, target)
                    + a.iter()
                        .map(|s| count_derefs_in_stmt(s, target))
                        .sum::<usize>()
                    + b.as_ref().map_or(0, |b| {
                        b.iter().map(|s| count_derefs_in_stmt(s, target)).sum()
                    })
            }
            Stmt::Switch(d, cases) => {
                count_derefs_in_expr(d, target)
                    + cases
                        .iter()
                        .map(|c| {
                            c.tests
                                .iter()
                                .map(|t| count_derefs_in_expr(t, target))
                                .sum::<usize>()
                                + c.body
                                    .iter()
                                    .map(|s| count_derefs_in_stmt(s, target))
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            }
            Stmt::Labeled(_, inner) => count_derefs_in_stmt(inner, target),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                b.iter().map(|s| count_derefs_in_stmt(s, target)).sum()
            }
            Stmt::While(t, b) => {
                count_derefs_in_expr(t, target)
                    + b.iter()
                        .map(|s| count_derefs_in_stmt(s, target))
                        .sum::<usize>()
            }
            Stmt::TryCatch { body, handler, .. } => {
                body.iter()
                    .map(|s| count_derefs_in_stmt(s, target))
                    .sum::<usize>()
                    + handler
                        .iter()
                        .map(|s| count_derefs_in_stmt(s, target))
                        .sum::<usize>()
            }
            _ => 0,
        }
    }

    fn expr_mutates_deps_or_calls(expr: &Expr, deps: &[&str], forbid_calls: bool) -> bool {
        match expr {
            Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                let writes_dep = match lhs.as_ref() {
                    Expr::Ident(n) => deps.contains(&n.as_str()),
                    Expr::Member(obj, prop) if prop == "buf" || prop == "off" => {
                        matches!(obj.as_ref(), Expr::Ident(n) if deps.contains(&n.as_str()))
                    }
                    _ => false,
                };
                writes_dep
                    || expr_mutates_deps_or_calls(lhs, deps, forbid_calls)
                    || expr_mutates_deps_or_calls(rhs, deps, forbid_calls)
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                forbid_calls
                    || expr_mutates_deps_or_calls(c, deps, forbid_calls)
                    || args
                        .iter()
                        .any(|a| expr_mutates_deps_or_calls(a, deps, forbid_calls))
            }
            Expr::Member(obj, _) => expr_mutates_deps_or_calls(obj, deps, forbid_calls),
            Expr::Index(obj, idx) => {
                expr_mutates_deps_or_calls(obj, deps, forbid_calls)
                    || expr_mutates_deps_or_calls(idx, deps, forbid_calls)
            }
            Expr::Unary(_, inner) => expr_mutates_deps_or_calls(inner, deps, forbid_calls),
            Expr::Binary(_, l, r) => {
                expr_mutates_deps_or_calls(l, deps, forbid_calls)
                    || expr_mutates_deps_or_calls(r, deps, forbid_calls)
            }
            Expr::Cond(c, t, e) => {
                expr_mutates_deps_or_calls(c, deps, forbid_calls)
                    || expr_mutates_deps_or_calls(t, deps, forbid_calls)
                    || expr_mutates_deps_or_calls(e, deps, forbid_calls)
            }
            Expr::Object(entries) => entries
                .iter()
                .any(|(_, v)| expr_mutates_deps_or_calls(v, deps, forbid_calls)),
            Expr::Array(items) | Expr::Seq(items) => items
                .iter()
                .any(|x| expr_mutates_deps_or_calls(x, deps, forbid_calls)),
            _ => false,
        }
    }

    fn stmt_mutates_deps_or_calls(stmt: &Stmt, deps: &[&str], forbid_calls: bool) -> bool {
        match stmt {
            Stmt::Let(n, init) => {
                deps.contains(&n.as_str())
                    || init
                        .as_ref()
                        .is_some_and(|e| expr_mutates_deps_or_calls(e, deps, forbid_calls))
            }
            Stmt::Const(n, e) => {
                deps.contains(&n.as_str()) || expr_mutates_deps_or_calls(e, deps, forbid_calls)
            }
            Stmt::Lets(decls) => decls.iter().any(|(n, init)| {
                deps.contains(&n.as_str())
                    || init
                        .as_ref()
                        .is_some_and(|e| expr_mutates_deps_or_calls(e, deps, forbid_calls))
            }),
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => {
                expr_mutates_deps_or_calls(e, deps, forbid_calls)
            }
            Stmt::If(t, a, b) => {
                expr_mutates_deps_or_calls(t, deps, forbid_calls)
                    || a.iter()
                        .any(|s| stmt_mutates_deps_or_calls(s, deps, forbid_calls))
                    || b.as_ref().is_some_and(|b| {
                        b.iter()
                            .any(|s| stmt_mutates_deps_or_calls(s, deps, forbid_calls))
                    })
            }
            Stmt::Switch(d, cases) => {
                expr_mutates_deps_or_calls(d, deps, forbid_calls)
                    || cases.iter().any(|c| {
                        c.tests
                            .iter()
                            .any(|t| expr_mutates_deps_or_calls(t, deps, forbid_calls))
                            || c.body
                                .iter()
                                .any(|s| stmt_mutates_deps_or_calls(s, deps, forbid_calls))
                    })
            }
            Stmt::Labeled(_, inner) => stmt_mutates_deps_or_calls(inner, deps, forbid_calls),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => b
                .iter()
                .any(|s| stmt_mutates_deps_or_calls(s, deps, forbid_calls)),
            Stmt::While(t, b) => {
                expr_mutates_deps_or_calls(t, deps, forbid_calls)
                    || b.iter()
                        .any(|s| stmt_mutates_deps_or_calls(s, deps, forbid_calls))
            }
            Stmt::TryCatch { body, handler, .. } => {
                body.iter()
                    .any(|s| stmt_mutates_deps_or_calls(s, deps, forbid_calls))
                    || handler
                        .iter()
                        .any(|s| stmt_mutates_deps_or_calls(s, deps, forbid_calls))
            }
            _ => false,
        }
    }

    fn replace_deref_in_expr(expr: &mut Expr, target: &str, replacement: &Expr) {
        if match_slot_deref(expr) == Some(target) {
            *expr = replacement.clone();
            return;
        }
        match expr {
            Expr::Member(obj, _) => replace_deref_in_expr(obj, target, replacement),
            Expr::Index(obj, idx) => {
                replace_deref_in_expr(obj, target, replacement);
                replace_deref_in_expr(idx, target, replacement);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                replace_deref_in_expr(c, target, replacement);
                for a in args {
                    replace_deref_in_expr(a, target, replacement);
                }
            }
            Expr::Unary(_, inner) => replace_deref_in_expr(inner, target, replacement),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                replace_deref_in_expr(l, target, replacement);
                replace_deref_in_expr(r, target, replacement);
            }
            Expr::Cond(c, t, e) => {
                replace_deref_in_expr(c, target, replacement);
                replace_deref_in_expr(t, target, replacement);
                replace_deref_in_expr(e, target, replacement);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    replace_deref_in_expr(v, target, replacement);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    replace_deref_in_expr(x, target, replacement);
                }
            }
            _ => {}
        }
    }

    fn replace_deref_in_stmt(stmt: &mut Stmt, target: &str, replacement: &Expr) {
        match stmt {
            Stmt::ExprStmt(e)
            | Stmt::Throw(e)
            | Stmt::Const(_, e)
            | Stmt::Let(_, Some(e))
            | Stmt::Return(Some(e)) => replace_deref_in_expr(e, target, replacement),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        replace_deref_in_expr(e, target, replacement);
                    }
                }
            }
            Stmt::If(t, a, b) => {
                replace_deref_in_expr(t, target, replacement);
                for s in a {
                    replace_deref_in_stmt(s, target, replacement);
                }
                if let Some(b) = b {
                    for s in b {
                        replace_deref_in_stmt(s, target, replacement);
                    }
                }
            }
            Stmt::Switch(d, cases) => {
                replace_deref_in_expr(d, target, replacement);
                for c in cases {
                    for t in &mut c.tests {
                        replace_deref_in_expr(t, target, replacement);
                    }
                    for s in &mut c.body {
                        replace_deref_in_stmt(s, target, replacement);
                    }
                }
            }
            Stmt::Labeled(_, inner) => replace_deref_in_stmt(inner, target, replacement),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                for s in b {
                    replace_deref_in_stmt(s, target, replacement);
                }
            }
            Stmt::While(t, b) => {
                replace_deref_in_expr(t, target, replacement);
                for s in b {
                    replace_deref_in_stmt(s, target, replacement);
                }
            }
            Stmt::TryCatch { body, handler, .. } => {
                for s in body {
                    replace_deref_in_stmt(s, target, replacement);
                }
                for s in handler {
                    replace_deref_in_stmt(s, target, replacement);
                }
            }
            _ => {}
        }
    }

    fn rewrite_stmt_children(stmt: &mut Stmt, eligible: &HashMap<String, usize>) {
        match stmt {
            Stmt::If(_, a, b) => {
                rewrite_block(a, eligible);
                if let Some(b) = b {
                    rewrite_block(b, eligible);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    rewrite_block(&mut c.body, eligible);
                }
            }
            Stmt::Labeled(_, inner) => rewrite_stmt_children(inner, eligible),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                rewrite_block(b, eligible);
            }
            Stmt::TryCatch { body, handler, .. } => {
                rewrite_block(body, eligible);
                rewrite_block(handler, eligible);
            }
            _ => {}
        }
    }

    fn rewrite_block(stmts: &mut Vec<Stmt>, eligible: &HashMap<String, usize>) {
        // First recurse into nested blocks.
        for stmt in stmts.iter_mut() {
            rewrite_stmt_children(stmt, eligible);
        }

        let mut i = 0;
        while i < stmts.len() {
            let matched = match &stmts[i] {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) => match lhs.as_ref() {
                    Expr::Ident(name) => eligible.get(name).and_then(|&total| {
                        match_slot_def(name, rhs).map(|def| (name.clone(), total, def))
                    }),
                    _ => None,
                },
                Stmt::Let(name, Some(rhs)) => eligible.get(name).and_then(|&total| {
                    match_slot_def(name, rhs).map(|def| (name.clone(), total, def))
                }),
                _ => None,
            };
            let Some((name, total_derefs, def)) = matched else {
                i += 1;
                continue;
            };
            let mut last_use = None;
            let mut local_derefs = 0usize;
            for (j, s) in stmts.iter().enumerate().skip(i + 1) {
                let c = count_derefs_in_stmt(s, &name);
                if c > 0 {
                    local_derefs += c;
                    last_use = Some(j);
                }
            }
            let Some(last) = last_use else {
                i += 1;
                continue;
            };
            if local_derefs != total_derefs {
                i += 1;
                continue;
            }
            let deps = def.deps();
            let forbid_calls = def.is_offset();
            let safe =
                (i + 1..=last).all(|j| !stmt_mutates_deps_or_calls(&stmts[j], &deps, forbid_calls));
            if !safe {
                i += 1;
                continue;
            }
            let repl = def.replacement();
            for s in &mut stmts[i + 1..=last] {
                replace_deref_in_stmt(s, &name, &repl);
            }
            stmts.remove(i);
        }
    }

    let mut usage = HashMap::new();
    scan_stmts(body, locals, false, &mut usage);
    let eligible: HashMap<String, usize> = usage
        .into_iter()
        .filter(|(_, u)| u.writes == 1 && u.slot_derefs >= 1 && u.other_uses == 0)
        .map(|(name, u)| (name, u.slot_derefs))
        .collect();
    if !eligible.is_empty() {
        rewrite_block(body, &eligible);
    }
}

/// Eliminates `__rt.sfp(ptr, len)` allocations on single-assignment locals whose only observable
/// reads are element dereferences `v.buf[...]` (and optionally `v.len` within the same block while
/// `len` is unmodified).
///
/// When `v` never escapes as a slice record and is never inspected when `len == 0` (every element
/// access `v.buf[...]` requires `len > 0`, where `ptr` is already a valid `{ buf, off }` slot),
/// rewriting `v = __rt.sfp(ptr, len)` to `v = ptr` avoids both the `__rt.sfp` call and the
/// `{ buf, off, len }` allocation — including on empty slices (`len == 0`) where `ptr` is a
/// dangling integer that the loop never dereferences.
pub(super) fn fold_sfp_deref_only(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
    #[derive(Default)]
    struct SfpUsage {
        writes: usize,
        index_uses: usize,
        len_reads: usize,
        other_uses: usize,
    }

    fn is_v_member<'a>(expr: &'a Expr, prop: &str) -> Option<&'a str> {
        match expr {
            Expr::Member(obj, p) if p == prop => match obj.as_ref() {
                Expr::Ident(name) => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    fn is_v_off_index(expr: &Expr, name: &str) -> bool {
        match expr {
            Expr::Member(obj, p) if p == "off" => {
                matches!(obj.as_ref(), Expr::Ident(n) if n == name)
            }
            Expr::Binary(jsast::BinOp::Add | jsast::BinOp::Sub, l, r) => {
                is_v_off_index(l, name) && !expr_mentions_ident(r, name)
            }
            _ => false,
        }
    }

    fn expr_mentions_ident(expr: &Expr, target: &str) -> bool {
        match expr {
            Expr::Ident(n) => n == target,
            Expr::Member(o, _) => expr_mentions_ident(o, target),
            Expr::Index(o, i) => expr_mentions_ident(o, target) || expr_mentions_ident(i, target),
            Expr::Call(c, args) | Expr::New(c, args) => {
                expr_mentions_ident(c, target)
                    || args.iter().any(|a| expr_mentions_ident(a, target))
            }
            Expr::Unary(_, e) => expr_mentions_ident(e, target),
            Expr::Binary(_, l, r) | Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) => {
                expr_mentions_ident(l, target) || expr_mentions_ident(r, target)
            }
            Expr::Cond(c, t, e) => {
                expr_mentions_ident(c, target)
                    || expr_mentions_ident(t, target)
                    || expr_mentions_ident(e, target)
            }
            Expr::Object(entries) => entries.iter().any(|(_, v)| expr_mentions_ident(v, target)),
            Expr::Array(items) | Expr::Seq(items) => {
                items.iter().any(|x| expr_mentions_ident(x, target))
            }
            _ => false,
        }
    }

    fn match_sfp_call(expr: &Expr) -> Option<(&Expr, &Expr)> {
        let Expr::Call(callee, args) = expr else {
            return None;
        };
        if args.len() != 2 {
            return None;
        }
        let Expr::Member(rt, method) = callee.as_ref() else {
            return None;
        };
        if matches!(rt.as_ref(), Expr::Ident(r) if r == "__rt") && method == "sfp" {
            Some((&args[0], &args[1]))
        } else {
            None
        }
    }

    fn scan_expr(
        expr: &Expr,
        locals: &HashSet<String>,
        in_closure: bool,
        usage: &mut HashMap<String, SfpUsage>,
    ) {
        if !in_closure {
            if let Expr::Index(obj, idx) = expr {
                if let Some(name) = is_v_member(obj, "buf") {
                    if locals.contains(name) && is_v_off_index(idx, name) {
                        usage.entry(name.to_owned()).or_default().index_uses += 1;
                        // Scan non-`name` parts of `idx` for other locals.
                        if let Expr::Binary(_, _, r) = idx.as_ref() {
                            scan_expr(r, locals, in_closure, usage);
                        }
                        return;
                    }
                }
            }
            if let Some(name) = is_v_member(expr, "len") {
                if locals.contains(name) {
                    usage.entry(name.to_owned()).or_default().len_reads += 1;
                    return;
                }
            }
        }
        match expr {
            Expr::Ident(n) => {
                if locals.contains(n) {
                    usage.entry(n.clone()).or_default().other_uses += 1;
                }
            }
            Expr::Assign(lhs, rhs) => {
                if !in_closure {
                    if let Expr::Index(obj, idx) = lhs.as_ref() {
                        if let Some(name) = is_v_member(obj, "buf") {
                            if locals.contains(name) && is_v_off_index(idx, name) {
                                usage.entry(name.to_owned()).or_default().index_uses += 1;
                                if let Expr::Binary(_, _, r) = idx.as_ref() {
                                    scan_expr(r, locals, in_closure, usage);
                                }
                                scan_expr(rhs, locals, in_closure, usage);
                                return;
                            }
                        }
                    }
                }
                scan_expr(lhs, locals, in_closure, usage);
                scan_expr(rhs, locals, in_closure, usage);
            }
            Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) => {
                scan_expr(l, locals, in_closure, usage);
                scan_expr(r, locals, in_closure, usage);
            }
            Expr::Member(o, _) => scan_expr(o, locals, in_closure, usage),
            Expr::Index(o, i) => {
                scan_expr(o, locals, in_closure, usage);
                scan_expr(i, locals, in_closure, usage);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                scan_expr(c, locals, in_closure, usage);
                for a in args {
                    scan_expr(a, locals, in_closure, usage);
                }
            }
            Expr::Unary(_, inner) => scan_expr(inner, locals, in_closure, usage),
            Expr::Cond(c, t, e) => {
                scan_expr(c, locals, in_closure, usage);
                scan_expr(t, locals, in_closure, usage);
                scan_expr(e, locals, in_closure, usage);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    scan_expr(v, locals, in_closure, usage);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    scan_expr(x, locals, in_closure, usage);
                }
            }
            Expr::Arrow(_, jsast::ArrowBody::Expr(b)) => scan_expr(b, locals, true, usage),
            Expr::Arrow(_, jsast::ArrowBody::Block(stmts)) => {
                scan_stmts(stmts, locals, true, usage)
            }
            _ => {}
        }
    }

    fn scan_stmts(
        stmts: &[Stmt],
        locals: &HashSet<String>,
        in_closure: bool,
        usage: &mut HashMap<String, SfpUsage>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                    if !in_closure
                        && matches!(lhs.as_ref(), Expr::Ident(n) if locals.contains(n)) =>
                {
                    let Expr::Ident(name) = lhs.as_ref() else {
                        unreachable!()
                    };
                    usage.entry(name.clone()).or_default().writes += 1;
                    scan_expr(rhs, locals, in_closure, usage);
                }
                Stmt::Let(name, Some(init)) if !in_closure && locals.contains(name) => {
                    usage.entry(name.clone()).or_default().writes += 1;
                    scan_expr(init, locals, in_closure, usage);
                }
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => scan_expr(e, locals, in_closure, usage),
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            if !in_closure && locals.contains(name) {
                                usage.entry(name.clone()).or_default().writes += 1;
                            }
                            scan_expr(init, locals, in_closure, usage);
                        }
                    }
                }
                Stmt::If(t, a, b) => {
                    scan_expr(t, locals, in_closure, usage);
                    scan_stmts(a, locals, in_closure, usage);
                    if let Some(b) = b {
                        scan_stmts(b, locals, in_closure, usage);
                    }
                }
                Stmt::Switch(d, cases) => {
                    scan_expr(d, locals, in_closure, usage);
                    for c in cases {
                        for t in &c.tests {
                            scan_expr(t, locals, in_closure, usage);
                        }
                        scan_stmts(&c.body, locals, in_closure, usage);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    scan_stmts(
                        std::slice::from_ref(inner.as_ref()),
                        locals,
                        in_closure,
                        usage,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    scan_stmts(b, locals, in_closure, usage);
                }
                Stmt::FunctionDecl { body: b, .. } => scan_stmts(b, locals, true, usage),
                Stmt::While(t, b) => {
                    scan_expr(t, locals, in_closure, usage);
                    scan_stmts(b, locals, in_closure, usage);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    scan_stmts(body, locals, in_closure, usage);
                    scan_stmts(handler, locals, in_closure, usage);
                }
                _ => {}
            }
        }
    }

    fn is_pure_len_expr(expr: &Expr) -> bool {
        match expr {
            Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) => true,
            Expr::Member(obj, _) => is_pure_len_expr(obj),
            _ => false,
        }
    }

    fn rewrite_stmts(stmts: &mut [Stmt], deref_only: &HashSet<String>) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if deref_only.contains(n)) => {
                    if let Some((ptr_arg, len_arg)) = match_sfp_call(rhs) {
                        if jsast::is_droppable(len_arg) || is_pure_len_expr(len_arg) {
                            **rhs = ptr_arg.clone();
                        }
                    }
                }
                Stmt::Let(name, Some(init)) if deref_only.contains(name) => {
                    if let Some((ptr_arg, len_arg)) = match_sfp_call(init) {
                        if jsast::is_droppable(len_arg) || is_pure_len_expr(len_arg) {
                            *init = ptr_arg.clone();
                        }
                    }
                }
                Stmt::If(_, a, b) => {
                    rewrite_stmts(a, deref_only);
                    if let Some(b) = b {
                        rewrite_stmts(b, deref_only);
                    }
                }
                Stmt::Switch(_, cases) => {
                    for c in cases {
                        rewrite_stmts(&mut c.body, deref_only);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    rewrite_stmts(std::slice::from_mut(inner.as_mut()), deref_only);
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                    rewrite_stmts(b, deref_only);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    rewrite_stmts(body, deref_only);
                    rewrite_stmts(handler, deref_only);
                }
                _ => {}
            }
        }
    }

    /// Replaces `v.len` reads in `stmts` with `len_expr` when `v = __rt.sfp(ptr, len_expr)` is
    /// defined immediately before (only pure, non-aliasing local statements allowed in between)
    /// and `len_expr` is a pure identifier/literal/member-chain whose root is unmodified.
    fn fold_immediate_sfp_len(
        stmts: &mut [Stmt],
        candidates: &HashSet<String>,
        folded_len_vars: &mut HashSet<String>,
    ) {
        fn collect_idents(expr: &Expr, out: &mut HashSet<String>) {
            match expr {
                Expr::Ident(n) => {
                    out.insert(n.clone());
                }
                Expr::Member(obj, _) => collect_idents(obj, out),
                _ => {}
            }
        }
        fn expr_is_side_effect_free(expr: &Expr) -> bool {
            match expr {
                Expr::Ident(_)
                | Expr::Num(_)
                | Expr::RawNum(_)
                | Expr::Str(_)
                | Expr::Bool(_)
                | Expr::Null
                | Expr::Undefined => true,
                Expr::Member(o, _) | Expr::Unary(_, o) => expr_is_side_effect_free(o),
                Expr::Binary(_, l, r) | Expr::Index(l, r) => {
                    expr_is_side_effect_free(l) && expr_is_side_effect_free(r)
                }
                Expr::Cond(c, t, e) => {
                    expr_is_side_effect_free(c)
                        && expr_is_side_effect_free(t)
                        && expr_is_side_effect_free(e)
                }
                _ => false,
            }
        }
        fn replace_v_len_in_expr(expr: &mut Expr, var: &str, replacement: &Expr) -> usize {
            if is_v_member(expr, "len") == Some(var) {
                *expr = replacement.clone();
                return 1;
            }
            match expr {
                Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) | Expr::Binary(_, l, r) => {
                    replace_v_len_in_expr(l, var, replacement)
                        + replace_v_len_in_expr(r, var, replacement)
                }
                Expr::Member(o, _) | Expr::Unary(_, o) => {
                    replace_v_len_in_expr(o, var, replacement)
                }
                Expr::Index(o, i) => {
                    replace_v_len_in_expr(o, var, replacement)
                        + replace_v_len_in_expr(i, var, replacement)
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    let mut n = replace_v_len_in_expr(c, var, replacement);
                    for a in args {
                        n += replace_v_len_in_expr(a, var, replacement);
                    }
                    n
                }
                Expr::Cond(c, t, e) => {
                    replace_v_len_in_expr(c, var, replacement)
                        + replace_v_len_in_expr(t, var, replacement)
                        + replace_v_len_in_expr(e, var, replacement)
                }
                Expr::Object(entries) => entries
                    .iter_mut()
                    .map(|(_, v)| replace_v_len_in_expr(v, var, replacement))
                    .sum(),
                Expr::Array(items) | Expr::Seq(items) => items
                    .iter_mut()
                    .map(|x| replace_v_len_in_expr(x, var, replacement))
                    .sum(),
                _ => 0,
            }
        }
        fn fold_stmts_linear(
            stmts: &mut [Stmt],
            candidates: &HashSet<String>,
            active: &mut HashMap<String, (Expr, HashSet<String>)>,
            folded_len_vars: &mut HashSet<String>,
        ) {
            fn fold_one_expr(
                expr: &mut Expr,
                candidates: &HashSet<String>,
                active: &mut HashMap<String, (Expr, HashSet<String>)>,
                folded_len_vars: &mut HashSet<String>,
            ) {
                // Handle comma-separated `Seq` of pure assignments sequentially.
                if let Expr::Seq(items) = expr {
                    for item in items.iter_mut() {
                        fold_one_expr(item, candidates, active, folded_len_vars);
                    }
                    return;
                }
                if let Expr::Assign(lhs, rhs) = expr {
                    // Replace `v.len` in RHS first.
                    for (var, (len_expr, _)) in active.iter() {
                        if replace_v_len_in_expr(rhs, var, len_expr) > 0 {
                            folded_len_vars.insert(var.clone());
                        }
                    }
                    if let Expr::Ident(target) = lhs.as_ref() {
                        // Invalidate any active entry depending on `target` or keyed by `target`.
                        active.retain(|var, (_, deps)| var != target && !deps.contains(target));
                        if candidates.contains(target) {
                            if let Some((_, len_arg)) = match_sfp_call(rhs) {
                                if is_pure_len_expr(len_arg) {
                                    let mut deps = HashSet::new();
                                    collect_idents(len_arg, &mut deps);
                                    active.insert(target.clone(), (len_arg.clone(), deps));
                                    return;
                                }
                            }
                        }
                        if !expr_is_side_effect_free(rhs) {
                            // A call or arbitrary effect may mutate `.len` fields on heap objects.
                            active.retain(|_, (le, _)| !matches!(le, Expr::Member(_, _)));
                        }
                        return;
                    }
                    // Property/index write: invalidate member-based lengths.
                    active.retain(|_, (le, _)| !matches!(le, Expr::Member(_, _)));
                    return;
                }
                for (var, (len_expr, _)) in active.iter() {
                    if replace_v_len_in_expr(expr, var, len_expr) > 0 {
                        folded_len_vars.insert(var.clone());
                    }
                }
                if !expr_is_side_effect_free(expr) {
                    active.clear();
                }
            }

            for stmt in stmts.iter_mut() {
                match stmt {
                    Stmt::ExprStmt(e) => fold_one_expr(e, candidates, active, folded_len_vars),
                    Stmt::Let(target, Some(init)) => {
                        for (var, (len_expr, _)) in active.iter() {
                            if replace_v_len_in_expr(init, var, len_expr) > 0 {
                                folded_len_vars.insert(var.clone());
                            }
                        }
                        active.retain(|var, (_, deps)| var != target && !deps.contains(target));
                        if candidates.contains(target) {
                            if let Some((_, len_arg)) = match_sfp_call(init) {
                                if is_pure_len_expr(len_arg) {
                                    let mut deps = HashSet::new();
                                    collect_idents(len_arg, &mut deps);
                                    active.insert(target.clone(), (len_arg.clone(), deps));
                                    continue;
                                }
                            }
                        }
                        if !expr_is_side_effect_free(init) {
                            active.retain(|_, (le, _)| !matches!(le, Expr::Member(_, _)));
                        }
                    }
                    Stmt::Block(b) => fold_stmts_linear(b, candidates, active, folded_len_vars),
                    Stmt::Labeled(_, inner) => {
                        let mut sub = HashMap::new();
                        fold_stmts_linear(
                            std::slice::from_mut(inner.as_mut()),
                            candidates,
                            &mut sub,
                            folded_len_vars,
                        );
                        active.clear();
                    }
                    Stmt::If(cond, a, b) => {
                        for (var, (len_expr, _)) in active.iter() {
                            if replace_v_len_in_expr(cond, var, len_expr) > 0 {
                                folded_len_vars.insert(var.clone());
                            }
                        }
                        let mut a_active = active.clone();
                        fold_stmts_linear(a, candidates, &mut a_active, folded_len_vars);
                        if let Some(b) = b {
                            let mut b_active = active.clone();
                            fold_stmts_linear(b, candidates, &mut b_active, folded_len_vars);
                        }
                        active.clear();
                    }
                    Stmt::Switch(disc, cases) => {
                        for (var, (len_expr, _)) in active.iter() {
                            if replace_v_len_in_expr(disc, var, len_expr) > 0 {
                                folded_len_vars.insert(var.clone());
                            }
                        }
                        for c in cases {
                            let mut c_active = active.clone();
                            fold_stmts_linear(
                                &mut c.body,
                                candidates,
                                &mut c_active,
                                folded_len_vars,
                            );
                        }
                        active.clear();
                    }
                    Stmt::LoopForever(_, b) => {
                        let mut sub = HashMap::new();
                        fold_stmts_linear(b, candidates, &mut sub, folded_len_vars);
                        active.clear();
                    }
                    Stmt::While(cond, b) => {
                        let mut sub = HashMap::new();
                        fold_stmts_linear(b, candidates, &mut sub, folded_len_vars);
                        let _ = cond;
                        active.clear();
                    }
                    Stmt::TryCatch { body, handler, .. } => {
                        let mut sub = HashMap::new();
                        fold_stmts_linear(body, candidates, &mut sub, folded_len_vars);
                        let mut hsub = HashMap::new();
                        fold_stmts_linear(handler, candidates, &mut hsub, folded_len_vars);
                        active.clear();
                    }
                    Stmt::Return(Some(e)) | Stmt::Throw(e) => {
                        for (var, (len_expr, _)) in active.iter() {
                            if replace_v_len_in_expr(e, var, len_expr) > 0 {
                                folded_len_vars.insert(var.clone());
                            }
                        }
                        active.clear();
                    }
                    _ => active.clear(),
                }
            }
        }

        let mut active = HashMap::new();
        fold_stmts_linear(stmts, candidates, &mut active, folded_len_vars);
    }

    /// Propagates single-assignment aliases of `__rt.sfp` locals (`_3 = $v12`) inside straight-line
    /// blocks when `_3` is never mentioned outside statements strictly following `_3 = $v12` (so
    /// it cannot carry across a loop back-edge) and neither `_3` nor `$v12` is reassigned or
    /// captured by a closure. This exposes `slice::Iter` (`_3 = $v12; _3$2 = $v12.len; ... _3.buf[...]`)
    /// directly to `fold_immediate_sfp_len` and `rewrite_stmts`.
    fn propagate_sfp_aliases(body: &mut Vec<Stmt>, locals: &HashSet<String>) {
        fn count_ident_mentions(stmts: &[Stmt], target: &str) -> usize {
            let mut n = 0usize;
            for stmt in stmts {
                jsast::visit_idents(stmt, &mut |id| {
                    if id == target {
                        n += 1;
                    }
                });
            }
            n
        }

        fn replace_ident_in_expr(expr: &mut Expr, from: &str, to: &str) {
            match expr {
                Expr::Ident(n) if n == from => *n = to.to_owned(),
                Expr::Member(o, _) | Expr::Unary(_, o) => replace_ident_in_expr(o, from, to),
                Expr::Index(a, b)
                | Expr::Binary(_, a, b)
                | Expr::Assign(a, b)
                | Expr::CompoundAssign(_, a, b) => {
                    replace_ident_in_expr(a, from, to);
                    replace_ident_in_expr(b, from, to);
                }
                Expr::Call(c, args) | Expr::New(c, args) => {
                    replace_ident_in_expr(c, from, to);
                    for a in args {
                        replace_ident_in_expr(a, from, to);
                    }
                }
                Expr::Cond(c, t, e) => {
                    replace_ident_in_expr(c, from, to);
                    replace_ident_in_expr(t, from, to);
                    replace_ident_in_expr(e, from, to);
                }
                Expr::Object(entries) => {
                    for (_, v) in entries {
                        replace_ident_in_expr(v, from, to);
                    }
                }
                Expr::Array(items) | Expr::Seq(items) => {
                    for x in items {
                        replace_ident_in_expr(x, from, to);
                    }
                }
                _ => {}
            }
        }

        fn replace_ident_in_stmt(stmt: &mut Stmt, from: &str, to: &str) {
            match stmt {
                Stmt::ExprStmt(e)
                | Stmt::Throw(e)
                | Stmt::Const(_, e)
                | Stmt::Let(_, Some(e))
                | Stmt::Return(Some(e)) => replace_ident_in_expr(e, from, to),
                Stmt::Lets(decls) => {
                    for (_, init) in decls {
                        if let Some(e) = init {
                            replace_ident_in_expr(e, from, to);
                        }
                    }
                }
                Stmt::If(t, a, b) => {
                    replace_ident_in_expr(t, from, to);
                    for s in a {
                        replace_ident_in_stmt(s, from, to);
                    }
                    if let Some(b) = b {
                        for s in b {
                            replace_ident_in_stmt(s, from, to);
                        }
                    }
                }
                Stmt::Switch(d, cases) => {
                    replace_ident_in_expr(d, from, to);
                    for c in cases {
                        for t in &mut c.tests {
                            replace_ident_in_expr(t, from, to);
                        }
                        for s in &mut c.body {
                            replace_ident_in_stmt(s, from, to);
                        }
                    }
                }
                Stmt::Labeled(_, inner) => replace_ident_in_stmt(inner, from, to),
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    for s in b {
                        replace_ident_in_stmt(s, from, to);
                    }
                }
                Stmt::While(t, b) => {
                    replace_ident_in_expr(t, from, to);
                    for s in b {
                        replace_ident_in_stmt(s, from, to);
                    }
                }
                Stmt::TryCatch { body, handler, .. } => {
                    for s in body {
                        replace_ident_in_stmt(s, from, to);
                    }
                    for s in handler {
                        replace_ident_in_stmt(s, from, to);
                    }
                }
                _ => {}
            }
        }

        fn has_closure_use(stmts: &[Stmt], target: &str) -> bool {
            fn in_expr(expr: &Expr, target: &str, in_fn: bool) -> bool {
                match expr {
                    Expr::Ident(n) => in_fn && n == target,
                    Expr::Arrow(_, jsast::ArrowBody::Expr(b)) => in_expr(b, target, true),
                    Expr::Arrow(_, jsast::ArrowBody::Block(s)) => in_stmts(s, target, true),
                    Expr::Member(o, _) | Expr::Unary(_, o) => in_expr(o, target, in_fn),
                    Expr::Index(a, b)
                    | Expr::Binary(_, a, b)
                    | Expr::Assign(a, b)
                    | Expr::CompoundAssign(_, a, b) => {
                        in_expr(a, target, in_fn) || in_expr(b, target, in_fn)
                    }
                    Expr::Call(c, args) | Expr::New(c, args) => {
                        in_expr(c, target, in_fn) || args.iter().any(|a| in_expr(a, target, in_fn))
                    }
                    Expr::Cond(c, t, e) => {
                        in_expr(c, target, in_fn)
                            || in_expr(t, target, in_fn)
                            || in_expr(e, target, in_fn)
                    }
                    Expr::Object(entries) => entries.iter().any(|(_, v)| in_expr(v, target, in_fn)),
                    Expr::Array(items) | Expr::Seq(items) => {
                        items.iter().any(|x| in_expr(x, target, in_fn))
                    }
                    _ => false,
                }
            }
            fn in_stmts(stmts: &[Stmt], target: &str, in_fn: bool) -> bool {
                stmts.iter().any(|s| match s {
                    Stmt::ExprStmt(e)
                    | Stmt::Throw(e)
                    | Stmt::Const(_, e)
                    | Stmt::Let(_, Some(e))
                    | Stmt::Return(Some(e)) => in_expr(e, target, in_fn),
                    Stmt::Lets(decls) => decls
                        .iter()
                        .filter_map(|(_, i)| i.as_ref())
                        .any(|e| in_expr(e, target, in_fn)),
                    Stmt::If(t, a, b) => {
                        in_expr(t, target, in_fn)
                            || in_stmts(a, target, in_fn)
                            || b.as_ref().is_some_and(|b| in_stmts(b, target, in_fn))
                    }
                    Stmt::Switch(d, cases) => {
                        in_expr(d, target, in_fn)
                            || cases.iter().any(|c| {
                                c.tests.iter().any(|t| in_expr(t, target, in_fn))
                                    || in_stmts(&c.body, target, in_fn)
                            })
                    }
                    Stmt::Labeled(_, inner) => in_stmts(std::slice::from_ref(inner), target, in_fn),
                    Stmt::LoopForever(_, b) | Stmt::Block(b) => in_stmts(b, target, in_fn),
                    Stmt::While(t, b) => in_expr(t, target, in_fn) || in_stmts(b, target, in_fn),
                    Stmt::TryCatch { body, handler, .. } => {
                        in_stmts(body, target, in_fn) || in_stmts(handler, target, in_fn)
                    }
                    Stmt::FunctionDecl { body, .. } => in_stmts(body, target, true),
                    _ => false,
                })
            }
            in_stmts(stmts, target, false)
        }

        fn process_block(
            stmts: &mut Vec<Stmt>,
            sfp_vars: &HashSet<String>,
            single_write_aliases: &HashSet<String>,
            total_mentions: &HashMap<String, usize>,
        ) {
            let mut active_sfp: HashSet<String> = HashSet::new();
            let mut i = 0;
            while i < stmts.len() {
                if let Stmt::ExprStmt(Expr::Assign(lhs, rhs)) = &stmts[i] {
                    if let Expr::Ident(target) = lhs.as_ref() {
                        if sfp_vars.contains(target) && match_sfp_call(rhs).is_some() {
                            active_sfp.insert(target.clone());
                            i += 1;
                            continue;
                        }
                        if let Expr::Ident(src) = rhs.as_ref() {
                            if single_write_aliases.contains(target) && active_sfp.contains(src) {
                                let after_mentions = count_ident_mentions(&stmts[i + 1..], target);
                                if total_mentions.get(target).copied() == Some(after_mentions + 1) {
                                    let from = target.clone();
                                    let to = src.clone();
                                    for s in &mut stmts[i + 1..] {
                                        replace_ident_in_stmt(s, &from, &to);
                                    }
                                    stmts.remove(i);
                                    continue;
                                }
                            }
                        }
                    }
                }
                i += 1;
            }
            for stmt in stmts.iter_mut() {
                match stmt {
                    Stmt::If(_, a, b) => {
                        process_block(a, sfp_vars, single_write_aliases, total_mentions);
                        if let Some(b) = b {
                            process_block(b, sfp_vars, single_write_aliases, total_mentions);
                        }
                    }
                    Stmt::Switch(_, cases) => {
                        for c in cases {
                            process_block(
                                &mut c.body,
                                sfp_vars,
                                single_write_aliases,
                                total_mentions,
                            );
                        }
                    }
                    Stmt::Labeled(_, inner) => {
                        if let Stmt::Block(b) = inner.as_mut() {
                            process_block(b, sfp_vars, single_write_aliases, total_mentions);
                        }
                    }
                    Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                        process_block(b, sfp_vars, single_write_aliases, total_mentions);
                    }
                    Stmt::TryCatch { body, handler, .. } => {
                        process_block(body, sfp_vars, single_write_aliases, total_mentions);
                        process_block(handler, sfp_vars, single_write_aliases, total_mentions);
                    }
                    _ => {}
                }
            }
        }

        let mut usage = HashMap::new();
        scan_stmts(body, locals, false, &mut usage);
        let sfp_vars: HashSet<String> = usage
            .iter()
            .filter(|(name, u)| u.writes == 1 && !has_closure_use(body, name))
            .map(|(name, _)| name.clone())
            .collect();
        let single_write_aliases: HashSet<String> = usage
            .iter()
            .filter(|(name, u)| {
                u.writes == 1
                    && (u.index_uses > 0 || u.len_reads > 0)
                    && u.other_uses == 0
                    && !has_closure_use(body, name)
            })
            .map(|(name, _)| name.clone())
            .collect();
        if single_write_aliases.is_empty() || sfp_vars.is_empty() {
            return;
        }
        let mut total_mentions: HashMap<String, usize> = HashMap::new();
        for stmt in body.iter() {
            jsast::visit_idents(stmt, &mut |id| {
                if single_write_aliases.contains(id) {
                    *total_mentions.entry(id.to_owned()).or_default() += 1;
                }
            });
        }
        process_block(body, &sfp_vars, &single_write_aliases, &total_mentions);
    }

    propagate_sfp_aliases(body, locals);

    let mut usage = HashMap::new();
    scan_stmts(body, locals, false, &mut usage);
    let len_candidates: HashSet<String> = usage
        .iter()
        .filter(|(_, u)| {
            u.writes == 1 && u.index_uses >= 1 && u.len_reads >= 1 && u.other_uses == 0
        })
        .map(|(name, _)| name.clone())
        .collect();
    if !len_candidates.is_empty() {
        let mut folded_len_vars = HashSet::new();
        fold_immediate_sfp_len(body, &len_candidates, &mut folded_len_vars);
        if !folded_len_vars.is_empty() {
            usage.clear();
            scan_stmts(body, locals, false, &mut usage);
        }
    }
    let deref_only: HashSet<String> = usage
        .into_iter()
        .filter(|(_, u)| {
            u.writes == 1 && u.index_uses >= 1 && u.len_reads == 0 && u.other_uses == 0
        })
        .map(|(name, _)| name)
        .collect();
    if !deref_only.is_empty() {
        rewrite_stmts(body, &deref_only);
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
pub(super) fn cse_slice_loads(
    body: &mut Vec<Stmt>,
    declarations: &mut Vec<(String, Option<Expr>)>,
    local_names: &mut HashSet<String>,
) {
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
            Expr::Member(obj, method) if matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt") => {
                matches!(method.as_str(), "bstr_bytes" | "str_bytes")
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

    fn expr_has_any_call(expr: &Expr) -> bool {
        match expr {
            Expr::Call(_, _) | Expr::New(_, _) => true,
            Expr::Member(o, _) | Expr::Unary(_, o) => expr_has_any_call(o),
            Expr::Index(a, b)
            | Expr::Binary(_, a, b)
            | Expr::Assign(a, b)
            | Expr::CompoundAssign(_, a, b) => expr_has_any_call(a) || expr_has_any_call(b),
            Expr::Cond(c, t, e) => {
                expr_has_any_call(c) || expr_has_any_call(t) || expr_has_any_call(e)
            }
            Expr::Object(entries) => entries.iter().any(|(_, v)| expr_has_any_call(v)),
            Expr::Array(items) | Expr::Seq(items) => items.iter().any(expr_has_any_call),
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
                if !matches!(op, jsast::BinOp::And | jsast::BinOp::Or) && !expr_has_any_call(l) {
                    collect_unconditional_in_expr(r, out);
                }
            }
            Expr::Cond(c, _, _) => collect_unconditional_in_expr(c, out),
            Expr::Index(o, i) => {
                collect_unconditional_in_expr(o, out);
                if !expr_has_any_call(o) {
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

    fn expr_invalidates(expr: &Expr, key: &SliceLoadKey) -> bool {
        match expr {
            Expr::Assign(lhs, rhs) | Expr::CompoundAssign(_, lhs, rhs) => {
                let lhs_bad = match lhs.as_ref() {
                    Expr::Ident(name) => key.mentions(name),
                    Expr::Index(_, _) => true,
                    _ => false,
                };
                lhs_bad || expr_invalidates(lhs, key) || expr_invalidates(rhs, key)
            }
            Expr::Call(callee, args) => {
                !is_non_mutating_call(callee)
                    || expr_invalidates(callee, key)
                    || args.iter().any(|a| expr_invalidates(a, key))
            }
            Expr::New(_, _) | Expr::Arrow(_, _) => true,
            Expr::Member(o, _) | Expr::Unary(_, o) => expr_invalidates(o, key),
            Expr::Index(a, b) | Expr::Binary(_, a, b) => {
                expr_invalidates(a, key) || expr_invalidates(b, key)
            }
            Expr::Cond(c, t, e) => {
                expr_invalidates(c, key) || expr_invalidates(t, key) || expr_invalidates(e, key)
            }
            Expr::Object(entries) => entries.iter().any(|(_, v)| expr_invalidates(v, key)),
            Expr::Array(items) | Expr::Seq(items) => items.iter().any(|x| expr_invalidates(x, key)),
            _ => false,
        }
    }

    fn stmt_invalidates(stmt: &Stmt, key: &SliceLoadKey) -> bool {
        match stmt {
            Stmt::Let(n, init) => {
                key.mentions(n) || init.as_ref().is_some_and(|e| expr_invalidates(e, key))
            }
            Stmt::Const(n, e) => key.mentions(n) || expr_invalidates(e, key),
            Stmt::Lets(decls) => decls.iter().any(|(n, init)| {
                key.mentions(n) || init.as_ref().is_some_and(|e| expr_invalidates(e, key))
            }),
            Stmt::ExprStmt(e) | Stmt::Throw(e) | Stmt::Return(Some(e)) => expr_invalidates(e, key),
            Stmt::If(test, a, b) => {
                expr_invalidates(test, key)
                    || a.iter().any(|s| stmt_invalidates(s, key))
                    || b.as_ref()
                        .is_some_and(|b| b.iter().any(|s| stmt_invalidates(s, key)))
            }
            Stmt::Switch(disc, cases) => {
                expr_invalidates(disc, key)
                    || cases.iter().any(|c| {
                        c.tests.iter().any(|t| expr_invalidates(t, key))
                            || c.body.iter().any(|s| stmt_invalidates(s, key))
                    })
            }
            Stmt::Labeled(_, inner) => stmt_invalidates(inner, key),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => b.iter().any(|s| stmt_invalidates(s, key)),
            Stmt::While(test, b) => {
                expr_invalidates(test, key) || b.iter().any(|s| stmt_invalidates(s, key))
            }
            Stmt::TryCatch { .. } | Stmt::FunctionDecl { .. } => true,
            _ => false,
        }
    }

    fn count_in_expr(expr: &Expr, key: &SliceLoadKey) -> usize {
        match expr {
            Expr::Member(obj, _) => {
                if match_slice_load(obj).as_ref() == Some(key) {
                    1
                } else {
                    count_in_expr(obj, key)
                }
            }
            Expr::Unary(_, inner) => count_in_expr(inner, key),
            Expr::Index(a, b)
            | Expr::Binary(_, a, b)
            | Expr::Assign(a, b)
            | Expr::CompoundAssign(_, a, b) => count_in_expr(a, key) + count_in_expr(b, key),
            Expr::Call(c, args) | Expr::New(c, args) => {
                count_in_expr(c, key) + args.iter().map(|a| count_in_expr(a, key)).sum::<usize>()
            }
            Expr::Cond(c, t, e) => {
                count_in_expr(c, key) + count_in_expr(t, key) + count_in_expr(e, key)
            }
            Expr::Object(entries) => entries.iter().map(|(_, v)| count_in_expr(v, key)).sum(),
            Expr::Array(items) | Expr::Seq(items) => {
                items.iter().map(|x| count_in_expr(x, key)).sum()
            }
            _ => 0,
        }
    }

    fn count_in_stmt(stmt: &Stmt, key: &SliceLoadKey) -> usize {
        match stmt {
            Stmt::ExprStmt(e)
            | Stmt::Throw(e)
            | Stmt::Const(_, e)
            | Stmt::Let(_, Some(e))
            | Stmt::Return(Some(e)) => count_in_expr(e, key),
            Stmt::Lets(decls) => decls
                .iter()
                .filter_map(|(_, init)| init.as_ref())
                .map(|e| count_in_expr(e, key))
                .sum(),
            Stmt::If(test, a, b) => {
                count_in_expr(test, key)
                    + a.iter().map(|s| count_in_stmt(s, key)).sum::<usize>()
                    + b.as_ref()
                        .map_or(0, |b| b.iter().map(|s| count_in_stmt(s, key)).sum())
            }
            Stmt::Switch(disc, cases) => {
                count_in_expr(disc, key)
                    + cases
                        .iter()
                        .map(|c| {
                            c.tests.iter().map(|t| count_in_expr(t, key)).sum::<usize>()
                                + c.body.iter().map(|s| count_in_stmt(s, key)).sum::<usize>()
                        })
                        .sum::<usize>()
            }
            Stmt::Labeled(_, inner) => count_in_stmt(inner, key),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                b.iter().map(|s| count_in_stmt(s, key)).sum()
            }
            Stmt::While(test, b) => {
                count_in_expr(test, key) + b.iter().map(|s| count_in_stmt(s, key)).sum::<usize>()
            }
            _ => 0,
        }
    }

    fn replace_in_expr(expr: &mut Expr, key: &SliceLoadKey, var: &str) {
        match expr {
            Expr::Member(obj, _) => {
                if match_slice_load(obj).as_ref() == Some(key) {
                    **obj = jsast::id(var.to_owned());
                } else {
                    replace_in_expr(obj, key, var);
                }
            }
            Expr::Unary(_, inner) => replace_in_expr(inner, key, var),
            Expr::Index(a, b)
            | Expr::Binary(_, a, b)
            | Expr::Assign(a, b)
            | Expr::CompoundAssign(_, a, b) => {
                replace_in_expr(a, key, var);
                replace_in_expr(b, key, var);
            }
            Expr::Call(c, args) | Expr::New(c, args) => {
                replace_in_expr(c, key, var);
                for a in args {
                    replace_in_expr(a, key, var);
                }
            }
            Expr::Cond(c, t, e) => {
                replace_in_expr(c, key, var);
                replace_in_expr(t, key, var);
                replace_in_expr(e, key, var);
            }
            Expr::Object(entries) => {
                for (_, v) in entries {
                    replace_in_expr(v, key, var);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for x in items {
                    replace_in_expr(x, key, var);
                }
            }
            _ => {}
        }
    }

    fn replace_in_stmt(stmt: &mut Stmt, key: &SliceLoadKey, var: &str) {
        match stmt {
            Stmt::ExprStmt(e)
            | Stmt::Throw(e)
            | Stmt::Const(_, e)
            | Stmt::Let(_, Some(e))
            | Stmt::Return(Some(e)) => replace_in_expr(e, key, var),
            Stmt::Lets(decls) => {
                for (_, init) in decls {
                    if let Some(e) = init {
                        replace_in_expr(e, key, var);
                    }
                }
            }
            Stmt::If(test, a, b) => {
                replace_in_expr(test, key, var);
                for s in a {
                    replace_in_stmt(s, key, var);
                }
                if let Some(b) = b {
                    for s in b {
                        replace_in_stmt(s, key, var);
                    }
                }
            }
            Stmt::Switch(disc, cases) => {
                replace_in_expr(disc, key, var);
                for c in cases {
                    for t in &mut c.tests {
                        replace_in_expr(t, key, var);
                    }
                    for s in &mut c.body {
                        replace_in_stmt(s, key, var);
                    }
                }
            }
            Stmt::Labeled(_, inner) => replace_in_stmt(inner, key, var),
            Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                for s in b {
                    replace_in_stmt(s, key, var);
                }
            }
            Stmt::While(test, b) => {
                replace_in_expr(test, key, var);
                for s in b {
                    replace_in_stmt(s, key, var);
                }
            }
            _ => {}
        }
    }

    fn fresh_name(
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

    fn process_block(
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
                    let c = count_in_stmt(s, &key);
                    if c > 0 {
                        total_count += c;
                        last_use = j;
                    }
                }
                if !valid_start || total_count < 2 {
                    continue;
                }
                let var = fresh_name(counter, declarations, local_names);
                for s in &mut stmts[i..=last_use] {
                    replace_in_stmt(s, &key, &var);
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
        for stmt in stmts.iter_mut() {
            process_children(stmt, counter, declarations, local_names);
        }
    }

    fn process_children(
        stmt: &mut Stmt,
        counter: &mut usize,
        declarations: &mut Vec<(String, Option<Expr>)>,
        local_names: &mut HashSet<String>,
    ) {
        match stmt {
            Stmt::If(_, a, b) => {
                process_block(a, counter, declarations, local_names);
                if let Some(b) = b {
                    process_block(b, counter, declarations, local_names);
                }
            }
            Stmt::Switch(_, cases) => {
                for c in cases {
                    process_block(&mut c.body, counter, declarations, local_names);
                }
            }
            Stmt::Labeled(_, inner) => process_children(inner, counter, declarations, local_names),
            Stmt::LoopForever(_, b) | Stmt::Block(b) | Stmt::While(_, b) => {
                process_block(b, counter, declarations, local_names);
            }
            Stmt::TryCatch { body, handler, .. } => {
                process_block(body, counter, declarations, local_names);
                process_block(handler, counter, declarations, local_names);
            }
            _ => {}
        }
    }

    let mut counter = 0usize;
    process_block(body, &mut counter, declarations, local_names);
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

    #[test]
    fn fold_offset_slot_derefs_folds_single_use_and_guards_mutations() {
        let mk_slot = |base: &str, idx: &str| {
            Expr::Cond(
                Box::new(Expr::Binary(
                    jsast::BinOp::StrictEq,
                    Box::new(jsast::id(idx)),
                    Box::new(jsast::num(0.0)),
                )),
                Box::new(jsast::id(base)),
                Box::new(Expr::Object(vec![
                    (
                        "buf".to_owned(),
                        Expr::Member(Box::new(jsast::id(base)), "buf".to_owned()),
                    ),
                    (
                        "off".to_owned(),
                        Expr::Binary(
                            jsast::BinOp::Add,
                            Box::new(Expr::Member(Box::new(jsast::id(base)), "off".to_owned())),
                            Box::new(jsast::id(idx)),
                        ),
                    ),
                ])),
            )
        };
        let slot_deref = |slot: &str| {
            Expr::Index(
                Box::new(Expr::Member(Box::new(jsast::id(slot)), "buf".to_owned())),
                Box::new(Expr::Member(Box::new(jsast::id(slot)), "off".to_owned())),
            )
        };
        let locals: HashSet<String> = ["slot".to_owned(), "out".to_owned()].into_iter().collect();

        // Positive: single-assignment offset slot folded into direct element read.
        let mut body = vec![
            jsast::assign_stmt(jsast::id("slot"), mk_slot("p", "i")),
            jsast::assign_stmt(jsast::id("out"), slot_deref("slot")),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_offset_slot_derefs(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(printed.contains("p.buf[p.off + i]"), "{printed}");
        assert!(!printed.contains("slot ="), "{printed}");

        // Negative: mutating `i` between def and deref must prevent folding.
        let mut mutated = vec![
            jsast::assign_stmt(jsast::id("slot"), mk_slot("p", "i")),
            jsast::assign_stmt(jsast::id("i"), jsast::num(9.0)),
            jsast::assign_stmt(jsast::id("out"), slot_deref("slot")),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_offset_slot_derefs(&mut mutated, &locals);
        let printed_mut = jsast::stmt_to_string(&Stmt::Block(mutated));
        assert!(printed_mut.contains("slot ="), "{printed_mut}");
        assert!(printed_mut.contains("slot.buf[slot.off]"), "{printed_mut}");
    }

    #[test]
    fn fold_sfp_deref_only_eliminates_sfp_for_deref_and_immediate_len() {
        let locals: HashSet<String> = ["s".to_owned(), "n".to_owned(), "out".to_owned()]
            .into_iter()
            .collect();
        let sfp_rhs = jsast::rt_call(
            "sfp",
            vec![
                Expr::Member(Box::new(jsast::id("v")), "ptr".to_owned()),
                Expr::Member(Box::new(jsast::id("v")), "len".to_owned()),
            ],
        );

        // Positive: `s = __rt.sfp(v.ptr, v.len); n = s.len; out = s.buf[s.off + i];`
        let mut body = vec![
            jsast::assign_stmt(jsast::id("s"), sfp_rhs.clone()),
            jsast::assign_stmt(
                jsast::id("n"),
                Expr::Member(Box::new(jsast::id("s")), "len".to_owned()),
            ),
            jsast::assign_stmt(
                jsast::id("out"),
                Expr::Index(
                    Box::new(Expr::Member(Box::new(jsast::id("s")), "buf".to_owned())),
                    Box::new(Expr::Binary(
                        jsast::BinOp::Add,
                        Box::new(Expr::Member(Box::new(jsast::id("s")), "off".to_owned())),
                        Box::new(jsast::id("i")),
                    )),
                ),
            ),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_sfp_deref_only(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("__rt.sfp"), "{printed}");
        assert!(printed.contains("s = v.ptr;"), "{printed}");
        assert!(printed.contains("n = v.len;"), "{printed}");

        // Negative: escaped `s` must keep `__rt.sfp`.
        let mut escaped = vec![
            jsast::assign_stmt(jsast::id("s"), sfp_rhs.clone()),
            Stmt::Return(Some(jsast::id("s"))),
        ];
        fold_sfp_deref_only(&mut escaped, &locals);
        let printed_esc = jsast::stmt_to_string(&Stmt::Block(escaped));
        assert!(printed_esc.contains("__rt.sfp"), "{printed_esc}");

        // Positive with single-assignment alias (`_3 = s; n = _3.len; out = _3.buf[_3.off + i]`).
        let locals_alias: HashSet<String> = [
            "s".to_owned(),
            "_3".to_owned(),
            "n".to_owned(),
            "out".to_owned(),
        ]
        .into_iter()
        .collect();
        let mut aliased = vec![
            jsast::assign_stmt(jsast::id("s"), sfp_rhs),
            jsast::assign_stmt(jsast::id("_3"), jsast::id("s")),
            jsast::assign_stmt(
                jsast::id("n"),
                Expr::Member(Box::new(jsast::id("_3")), "len".to_owned()),
            ),
            jsast::assign_stmt(
                jsast::id("out"),
                Expr::Index(
                    Box::new(Expr::Member(Box::new(jsast::id("_3")), "buf".to_owned())),
                    Box::new(Expr::Binary(
                        jsast::BinOp::Add,
                        Box::new(Expr::Member(Box::new(jsast::id("_3")), "off".to_owned())),
                        Box::new(jsast::id("i")),
                    )),
                ),
            ),
            Stmt::Return(Some(jsast::id("out"))),
        ];
        fold_sfp_deref_only(&mut aliased, &locals_alias);
        let printed_alias = jsast::stmt_to_string(&Stmt::Block(aliased));
        assert!(!printed_alias.contains("__rt.sfp"), "{printed_alias}");
        assert!(printed_alias.contains("n = v.len;"), "{printed_alias}");
    }

    #[test]
    fn eliminate_dead_local_writes_strips_unread_seq_element() {
        let mut body = vec![
            Stmt::ExprStmt(Expr::Seq(vec![
                Expr::Assign(Box::new(jsast::id("dead")), Box::new(jsast::num(1.0))),
                Expr::Assign(Box::new(jsast::id("live")), Box::new(jsast::num(2.0))),
            ])),
            Stmt::Return(Some(jsast::id("live"))),
        ];
        let locals: HashSet<String> = ["dead".to_owned(), "live".to_owned()].into_iter().collect();
        eliminate_dead_local_writes(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("dead"), "{printed}");
        assert!(printed.contains("live = 2;"), "{printed}");
    }

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
}
