//! One item as a top level declaration: a function (`base.rs::codegen_fn` and `codegen_body`),
//! a `static` (`base.rs::codegen_static`), or a vtable (`vtable.rs`).

use std::collections::{BTreeSet, HashSet};

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::{Decline, OpClass};
use rcgjs_object::jir::repr::Func;
use rcgjs_object::jir::semantic::{BodyKind, VtableEntry};
use rcgjs_object::jsast::{self, Stmt};
use rcgjs_object::ptr;
use structurizer::Region;

use crate::opts::OptimizerPass;

use super::super::plan::{BlockPayload, Plan, RScrut};
use super::super::{LowerInput, LoweredItem};
use super::{decline, emit, value, JsCx};

/// Lowers a function body whose SSA form, plan and region tree are in hand.
pub fn lower(
    input: &LowerInput<'_>,
    func: &Func,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
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
    .with_call_retargets(input.call_retargets);

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
    let mut param_names: HashSet<String> = params.iter().cloned().collect();
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
        if plan.param_cells.contains_key(cell) || param_names.contains(name) {
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
        eliminate_dead_local_writes(&mut body, &local_names);
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
    finish(input, decl, Vec::new(), cx.take_zombies(), Vec::new())
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
    finish(input, decl, Vec::new(), Vec::new(), Vec::new())
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
/// led by the item's own debug path, the root location.
fn finish(
    input: &LowerInput<'_>,
    decl: Stmt,
    originals: Vec<(String, String)>,
    zombies: Vec<rcgjs_object::item::FlatZombie>,
    js_alloc_origins: Vec<crate::analysis::js_alloc::Origin>,
) -> Result<LoweredItem, Decline> {
    // A baseline item passes through the shared finished-JavaScript SROA and DCE pipeline in
    // `JsItem::new`. JIR replacements are installed directly by the linker, so run the same
    // passes here: the semantic optimizer has already done the typed work, but final structuring
    // can expose one last object literal that is only read field by field.
    let decl = rcgjs_object::sroa::analyze_jir(decl, input.opts.optimizer).decl;
    let decl = rcgjs_object::optimizer::analyze(decl, input.opts.optimizer).decl;
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
}
