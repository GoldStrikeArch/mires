//! One item as a top level declaration: a function (`base.rs::codegen_fn` and `codegen_body`),
//! a `static` (`base.rs::codegen_static`), or a vtable (`vtable.rs`).

use std::collections::{BTreeSet, HashMap, HashSet};

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::{Decline, OpClass};
use rcgjs_object::jir::repr::Func;
use rcgjs_object::jir::semantic::{BodyKind, BodySemantic, VtableEntry};
use rcgjs_object::jsast::{
    self, walk_expr, walk_expr_mut, walk_stmt, walk_stmt_mut, ArrowBody, Expr, Stmt, Visit,
    VisitMut,
};
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
    if let Some(heap) = input.heap {
        if let BodySemantic::DropGlue(ty) = input.body.semantic {
            let is_noop_str = heap.native_string_enabled
                && input.table.get(ty).is_some_and(|d| {
                    d.meta.candidate == rcgjs_object::jir::types::RepresentationCandidate::String
                });
            let is_noop_vec = heap.native_vec_enabled
                && !heap.non_rc_raw_alloc_reachable
                && crate::analysis::heap::is_direct_global_vec_ty(input.table, ty)
                && !crate::analysis::heap::ty_effectively_needs_drop(
                    input.table,
                    ty,
                    heap.native_string_enabled,
                    true,
                );
            if is_noop_str || is_noop_vec {
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
    let off_zero_slice_positions =
        physical.map_or_else(Vec::new, |sig| sig.off_zero_slice_jir_positions());
    let byte_str_positions = physical.map_or_else(Vec::new, |sig| sig.byte_str_jir_positions());
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
        let slice = slice_positions.contains(&position)
            || byte_str_positions.iter().any(|&(pos, mode)| {
                pos == position && mode == crate::plan::abi_flat::ByteStrParam::Sub
            });
        if off_zero_slice_positions.contains(&position) {
            let buf = format!("{name}$buf");
            let len = format!("{name}$len");
            params.extend([buf.clone(), len.clone()]);
            logical_param_names.push(name.to_owned());
            cx.bind_expr(
                *param,
                ptr::fat_slice(jsast::id(buf), jsast::num(0), jsast::id(len)),
            );
        } else if slice {
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

    // Fail-closed verification: every `ByteStrParam` parameter promised by `target_abi` MUST
    // survive `byte_view_analysis` with the exact physical mode (`Full => FullSlice`,
    // `Sub => VarSlice`); otherwise decline so the lane retries cleanly.
    for &(pos, bs_mode) in &byte_str_positions {
        let Some(&param) = func.params.get(pos) else {
            return Err(decline(
                OpClass::Other,
                format!("byte_str param position {pos} out of bounds"),
            ));
        };
        let expected = match bs_mode {
            crate::plan::abi_flat::ByteStrParam::Full => super::ByteViewMode::FullSlice,
            crate::plan::abi_flat::ByteStrParam::Sub => super::ByteViewMode::VarSlice,
        };
        if cx.byte_view(param).map(|bv| bv.mode) != Some(expected) {
            return Err(decline(
                OpClass::Other,
                format!("byte_str param {pos} did not survive byte_view_analysis as {expected:?}"),
            ));
        }
    }

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
        collapse_ephemeral_rt_box_deref(&mut body, &uninit_locals);
        eliminate_dead_local_writes(&mut body, &local_names);
        fuse_local_bytes_str_with(&mut body, &uninit_locals, cx.needs_byte_slice_ref());
        fold_zero_offset_arith(&mut body, &uninit_locals);
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
        unbox_single_slot_array_locals(&mut body, &mut declarations);
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
    let mut immutable_slices: HashSet<String> = HashSet::new();
    for &param in &func.params {
        let Ok(ty) = cx.ty_of(param) else { continue };
        if input.table.pointer_form(ty) != Some(rcgjs_object::jir::types::PointerForm::FatSlice) {
            continue;
        }
        let Some(name) = cx.name_of(param) else {
            continue;
        };
        let is_boxed_param = plan
            .param_cells
            .iter()
            .any(|(cell, p)| *p == param && cx.cell_boxed(*cell));
        if !is_boxed_param && formal_param_names.contains(name) {
            immutable_slices.insert(name.to_owned());
        }
    }
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
        &immutable_slices,
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
            &HashSet::new(),
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
        &HashSet::new(),
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
    finish(
        input,
        decl,
        Vec::new(),
        cx.take_zombies(),
        Vec::new(),
        None,
        &HashSet::new(),
    )
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
    finish(
        input,
        decl,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        &HashSet::new(),
    )
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
    immutable_slices: &HashSet<String>,
) -> Result<LoweredItem, Decline> {
    // A baseline item passes through the shared finished-JavaScript SROA and DCE pipeline in
    // `JsItem::new`. JIR replacements are installed directly by the linker, so run the same
    // passes here: the semantic optimizer has already done the typed work, but final structuring
    // can expose one last object literal that is only read field by field.
    let decl = rcgjs_object::sroa::analyze_jir(decl, input.opts.optimizer).decl;
    let mut decl = rcgjs_object::optimizer::analyze(decl, input.opts.optimizer).decl;
    if input.opts.applies(OptimizerPass::AstDce) {
        if let Stmt::FunctionDecl { params, body, .. } = &mut decl {
            let mut local_names: HashSet<String> = params.iter().cloned().collect();
            let mut uninit_locals: HashSet<String> = HashSet::new();
            super::ast_util::collect_top_locals(body, &mut local_names, &mut uninit_locals);
            fold_zero_offset_arith(body, &uninit_locals);
            fold_offset_slot_derefs(body, &uninit_locals);
            eliminate_dead_local_writes(body, &local_names);
            fold_sfp_deref_only(body, &uninit_locals);
            eliminate_dead_local_writes(body, &local_names);
        }
        if input.opts.profile == Some(crate::opts::Profile::ReleaseSpeed) {
            super::post_opt::run_speed_post_opts_with_mode(
                &mut decl,
                input.final_lowering,
                immutable_slices,
            );
            if let Stmt::FunctionDecl { params, body, .. } = &mut decl {
                let mut local_names: HashSet<String> = params.iter().cloned().collect();
                let mut uninit_locals: HashSet<String> = HashSet::new();
                super::ast_util::collect_top_locals(body, &mut local_names, &mut uninit_locals);
                fold_zero_offset_arith(body, &uninit_locals);
                eliminate_dead_local_writes(body, &local_names);
            }
        }
    }
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

struct FuseReadCounter<'a> {
    target: &'a str,
    policy: &'a LocalFusePolicy,
    total: usize,
}

impl Visit for FuseReadCounter<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if matches!(stmt, Stmt::FunctionDecl { .. }) {
            return;
        }
        walk_stmt(self, stmt);
    }
    fn visit_expr(&mut self, expr: &Expr) {
        if let Some((_, arg)) = (self.policy.read_call)(expr) {
            self.total += usize::from(arg == self.target);
            return;
        }
        if matches!(expr, Expr::Arrow(_, _)) {
            return;
        }
        walk_expr(self, expr);
    }
}

fn count_reads_in_expr(expr: &Expr, target: &str, policy: &LocalFusePolicy) -> usize {
    let mut c = FuseReadCounter {
        target,
        policy,
        total: 0,
    };
    c.visit_expr(expr);
    c.total
}

fn count_reads_in_stmts(stmts: &[Stmt], target: &str, policy: &LocalFusePolicy) -> usize {
    let mut c = FuseReadCounter {
        target,
        policy,
        total: 0,
    };
    c.visit_stmts(stmts);
    c.total
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

#[derive(Default)]
struct LocalFuseUsage {
    writes: usize,
    reads: usize,
    str_bytes_reads: usize,
    disqualified: bool,
}

fn scan_local_fuse_usage(
    stmts: &[Stmt],
    locals: &HashSet<String>,
    policy: &LocalFusePolicy,
) -> HashMap<String, LocalFuseUsage> {
    struct Scan<'a> {
        locals: &'a HashSet<String>,
        policy: &'a LocalFusePolicy,
        usage: HashMap<String, LocalFuseUsage>,
    }
    impl Scan<'_> {
        fn record_write(&mut self, name: &str, rhs: &Expr) {
            let entry = self.usage.entry(name.to_owned()).or_default();
            if (self.policy.peel_rhs)(rhs) {
                entry.writes += 1;
            } else {
                entry.disqualified = true;
            }
        }
        fn scan_nested_scope(&mut self, params: &[String], f: impl FnOnce(&mut Scan<'_>)) {
            let shadow: HashSet<String> = self
                .locals
                .iter()
                .filter(|n| !params.contains(*n))
                .cloned()
                .collect();
            let mut inner = Scan {
                locals: &shadow,
                policy: self.policy,
                usage: HashMap::new(),
            };
            f(&mut inner);
            for (name, u) in inner.usage {
                if u.writes > 0 || u.reads > 0 || u.disqualified {
                    self.usage.entry(name).or_default().disqualified = true;
                }
            }
        }
    }
    impl Visit for Scan<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if self.locals.contains(n)) =>
                {
                    let Expr::Ident(name) = lhs.as_ref() else {
                        unreachable!()
                    };
                    self.record_write(name, rhs);
                    self.visit_expr(rhs);
                }
                Stmt::Let(name, Some(init)) if self.locals.contains(name) => {
                    self.record_write(name, init);
                    self.visit_expr(init);
                }
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            if self.locals.contains(name) {
                                self.record_write(name, init);
                            }
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
                        self.usage.entry(binding.clone()).or_default().disqualified = true;
                    }
                    self.visit_stmts(body);
                    self.visit_stmts(handler);
                }
                Stmt::FunctionDecl { params, body, .. } => {
                    self.scan_nested_scope(params, |inner| inner.visit_stmts(body));
                }
                _ => walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, expr: &Expr) {
            if let Some((method, arg)) = (self.policy.read_call)(expr) {
                if self.locals.contains(arg) {
                    let entry = self.usage.entry(arg.to_owned()).or_default();
                    entry.reads += 1;
                    if method == "str_bytes" {
                        entry.str_bytes_reads += 1;
                    }
                    return;
                }
            }
            match expr {
                Expr::Ident(name) if self.locals.contains(name) => {
                    self.usage.entry(name.clone()).or_default().disqualified = true;
                }
                Expr::Arrow(params, body) => {
                    self.scan_nested_scope(params, |inner| match body {
                        ArrowBody::Expr(b) => inner.visit_expr(b),
                        ArrowBody::Block(s) => inner.visit_stmts(s),
                    });
                }
                _ => walk_expr(self, expr),
            }
        }
    }
    let mut scan = Scan {
        locals,
        policy,
        usage: HashMap::new(),
    };
    scan.visit_stmts(stmts);
    scan.usage
}

fn rewrite_local_fuse(
    stmts: &mut [Stmt],
    eligible: &HashSet<String>,
    rewrite_rhs: fn(&mut Expr),
    rewrite_read: impl Fn(&mut Expr, &HashSet<String>) -> bool,
) {
    struct Rewriter<'a, F> {
        eligible: &'a HashSet<String>,
        rewrite_rhs: fn(&mut Expr),
        rewrite_read: F,
    }
    impl<F: Fn(&mut Expr, &HashSet<String>) -> bool> VisitMut for Rewriter<'_, F> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if self.eligible.contains(n)) =>
                {
                    self.visit_expr_mut(rhs);
                    (self.rewrite_rhs)(rhs);
                }
                Stmt::Let(name, Some(init)) if self.eligible.contains(name) => {
                    self.visit_expr_mut(init);
                    (self.rewrite_rhs)(init);
                }
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            self.visit_expr_mut(init);
                            if self.eligible.contains(name) {
                                (self.rewrite_rhs)(init);
                            }
                        }
                    }
                }
                Stmt::FunctionDecl { .. } => {}
                _ => walk_stmt_mut(self, stmt),
            }
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if (self.rewrite_read)(expr, self.eligible) || matches!(expr, Expr::Arrow(_, _)) {
                return;
            }
            walk_expr_mut(self, expr);
        }
    }
    Rewriter {
        eligible,
        rewrite_rhs,
        rewrite_read,
    }
    .visit_stmts_mut(stmts);
}

pub(super) fn fuse_local_bytes_str_with(
    body: &mut [Stmt],
    locals: &HashSet<String>,
    wrap_byte_slice_ref: bool,
) {
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

    let policy = LocalFusePolicy {
        peel_rhs: peel_bytes_str,
        read_call: str_view_call_target,
        allow_early_exit: true,
    };
    let usage = scan_local_fuse_usage(body, locals, &policy);
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
        rewrite_local_fuse(body, &eligible, rewrite_bytes_str_rhs, |expr, eligible| {
            let Some((method, arg)) = str_view_call_target(expr) else {
                return false;
            };
            if !eligible.contains(arg) {
                return false;
            }
            let arg = arg.to_owned();
            *expr = if method == "str_bytes" {
                if wrap_byte_slice_ref {
                    jsast::rt_call("byte_slice_ref", vec![Expr::Ident(arg)])
                } else {
                    Expr::Ident(arg)
                }
            } else {
                ptr::slot_len(Expr::Ident(arg))
            };
            true
        });
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
    struct Folder;
    impl VisitMut for Folder {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            walk_expr_mut(self, expr);
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
    }
    Folder.visit_stmts_mut(stmts);
}

/// Fuses single-write `x = __rt.bstr_in(host)` whose sole read in the body is `__rt.str_bytes(x)`
/// into `x = host; ... __rt.bstr_utf8_bytes(x)` only when the write dominates the read with no
/// intervening side effects, branches, or early exits so `__rt.bstr_in`'s `TypeError` check runs at
/// the same observable point via `__rt.bstr_utf8_bytes`.
fn fuse_local_bstr_in_bytes(body: &mut [Stmt], locals: &HashSet<String>) {
    fn peel_bstr_in(expr: &Expr) -> bool {
        seq_tail(expr).is_some_and(|tail| rt_unary_call(tail, "bstr_in").is_some())
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

    let policy = LocalFusePolicy {
        peel_rhs: peel_bstr_in,
        read_call: str_bytes_call_target,
        allow_early_exit: false,
    };
    let usage = scan_local_fuse_usage(body, locals, &policy);
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
        rewrite_local_fuse(body, &eligible, unwrap_bstr_in_rhs, |expr, eligible| {
            let Some(Expr::Ident(arg)) = rt_unary_call(expr, "str_bytes") else {
                return false;
            };
            if !eligible.contains(arg) {
                return false;
            }
            *expr = jsast::rt_call("bstr_utf8_bytes", vec![Expr::Ident(arg.clone())]);
            true
        });
    }
}

/// Whether `expr` is strictly side-effect-free and cannot throw even on `null`/`undefined`
/// (unlike property/index access), so a bare expression statement `expr;` can be dropped safely.
fn is_strictly_pure_expr(expr: &Expr) -> bool {
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
        Expr::Unary(_, inner) => is_strictly_pure_expr(inner),
        Expr::Binary(_, lhs, rhs) => is_strictly_pure_expr(lhs) && is_strictly_pure_expr(rhs),
        Expr::Cond(c, t, e) => {
            is_strictly_pure_expr(c) && is_strictly_pure_expr(t) && is_strictly_pure_expr(e)
        }
        Expr::Array(items) | Expr::Seq(items) => items.iter().all(is_strictly_pure_expr),
        Expr::Object(entries) => entries.iter().all(|(_, v)| is_strictly_pure_expr(v)),
        Expr::Member(..)
        | Expr::Index(..)
        | Expr::Call(..)
        | Expr::New(..)
        | Expr::Assign(..)
        | Expr::CompoundAssign(..)
        | Expr::Raw(_) => false,
    }
}

/// Removes assignments to compiler locals that no expression ever reads, plus strictly-pure bare
/// expression statements (such as dead `select_unpredictable` ternaries `cond ? a : b;`). JIR
/// values are SSA, but range/projection cleanup and lowered hints can leave pure expressions
/// behind after the last JIR DCE run. At this point the structured JavaScript is authoritative, so
/// a function-wide zero-read test is both smaller and more exact than trying to predict the
/// structurizer's final spelling.
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
        if !remove_unread_writes(body, &unread) {
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
            other if is_strictly_pure_expr(&other) => {
                *changed = true;
                None
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
            Stmt::TryCatch { body, handler, .. } => {
                changed |= remove_unread_writes(body, unread);
                changed |= remove_unread_writes(handler, unread);
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

/// Collapses ephemeral `(tmp = __rt.box(inner), tmp.buf[tmp.off])` expressions into `inner` when
/// `tmp` is an uninitialized compiler temporary used nowhere else in the function body.
fn collapse_ephemeral_rt_box_deref(body: &mut [Stmt], locals: &HashSet<String>) {
    let mut mentions: HashMap<String, usize> = HashMap::new();
    for stmt in body.iter() {
        jsast::visit_idents(stmt, &mut |name| {
            *mentions.entry(name.to_owned()).or_default() += 1;
        });
    }

    fn is_rt_box_call(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Call(callee, args)
                if args.len() == 1
                    && matches!(
                        callee.as_ref(),
                        Expr::Member(obj, method)
                            if method == "box"
                                && matches!(obj.as_ref(), Expr::Ident(rt) if rt == "__rt")
                    )
        )
    }

    fn is_slot_deref_of(expr: &Expr, tmp: &str) -> bool {
        let Expr::Index(buf, off) = expr else {
            return false;
        };
        matches!(
            (buf.as_ref(), off.as_ref()),
            (Expr::Member(b_obj, b_prop), Expr::Member(o_obj, o_prop))
                if b_prop == "buf"
                    && o_prop == "off"
                    && matches!(b_obj.as_ref(), Expr::Ident(a) if a == tmp)
                    && matches!(o_obj.as_ref(), Expr::Ident(b) if b == tmp)
        )
    }

    struct Rewriter<'a> {
        locals: &'a HashSet<String>,
        mentions: &'a HashMap<String, usize>,
    }
    impl VisitMut for Rewriter<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if matches!(stmt, Stmt::FunctionDecl { .. }) {
                return;
            }
            walk_stmt_mut(self, stmt);
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            walk_expr_mut(self, expr);
            if let Expr::Seq(items) = expr {
                if items.len() >= 2 {
                    let n = items.len();
                    let can_collapse = match (&items[n - 2], &items[n - 1]) {
                        (Expr::Assign(lhs, rhs), tail) => matches!(
                            lhs.as_ref(),
                            Expr::Ident(tmp)
                                if self.locals.contains(tmp)
                                    && self.mentions.get(tmp).copied() == Some(3)
                                    && is_rt_box_call(rhs)
                                    && is_slot_deref_of(tail, tmp)
                        ),
                        _ => false,
                    };
                    if can_collapse {
                        items.pop();
                        let assign = items.pop().expect("assign");
                        if let Expr::Assign(_, rhs) = assign {
                            if let Expr::Call(_, mut args) = *rhs {
                                let inner = args.pop().expect("1 arg");
                                items.push(inner);
                            }
                        }
                        if items.len() == 1 {
                            *expr = items.pop().expect("single");
                        }
                    }
                }
            }
        }
    }

    Rewriter {
        locals,
        mentions: &mentions,
    }
    .visit_stmts_mut(body);
}

/// Unboxes compiler-declared single-slot array cells (`let m = [];`) into plain scalars (`let m;`)
/// when every mention of `m` in `body` is strictly `m[0]` (read or write) outside any closure or
/// nested function. This cleans up boxed cells whose `{ buf: m, off: 0 }` views became dead after
/// lowering (such as `core::hint::select_unpredictable` in `slice::partition_point`).
fn unbox_single_slot_array_locals(body: &mut [Stmt], declarations: &mut [(String, Option<Expr>)]) {
    let candidates: HashSet<String> = declarations
        .iter()
        .filter(|(_, init)| matches!(init, Some(Expr::Array(items)) if items.is_empty()))
        .map(|(name, _)| name.clone())
        .collect();
    if candidates.is_empty() {
        return;
    }

    fn is_zero_index(expr: &Expr) -> bool {
        match expr {
            Expr::Num(n) => *n == 0.0,
            Expr::RawNum(s) => s == "0",
            _ => false,
        }
    }

    struct Scan<'a> {
        candidates: &'a HashSet<String>,
        in_closure: bool,
        disqualified: HashSet<String>,
        slot_uses: HashSet<String>,
    }
    impl Visit for Scan<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Const(name, _) | Stmt::Let(name, _) if self.candidates.contains(name) => {
                    self.disqualified.insert(name.clone());
                }
                Stmt::Lets(decls) => {
                    for (name, _) in decls {
                        if self.candidates.contains(name) {
                            self.disqualified.insert(name.clone());
                        }
                    }
                }
                Stmt::TryCatch { binding, .. } if self.candidates.contains(binding) => {
                    self.disqualified.insert(binding.clone());
                }
                Stmt::FunctionDecl { body, .. } => {
                    let prev = self.in_closure;
                    self.in_closure = true;
                    self.visit_stmts(body);
                    self.in_closure = prev;
                    return;
                }
                _ => {}
            }
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &Expr) {
            if !self.in_closure {
                if let Expr::Index(obj, idx) = expr {
                    if let Expr::Ident(name) = obj.as_ref() {
                        if self.candidates.contains(name) && is_zero_index(idx) {
                            self.slot_uses.insert(name.clone());
                            return;
                        }
                    }
                }
            }
            match expr {
                Expr::Ident(name) if self.candidates.contains(name) => {
                    self.disqualified.insert(name.clone());
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

    struct Rewriter<'a>(&'a HashSet<String>);
    impl VisitMut for Rewriter<'_> {
        fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
            if matches!(stmt, Stmt::FunctionDecl { .. }) {
                return;
            }
            walk_stmt_mut(self, stmt);
        }
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            if let Expr::Index(obj, idx) = expr {
                if let Expr::Ident(name) = obj.as_ref() {
                    if self.0.contains(name) && is_zero_index(idx) {
                        *expr = Expr::Ident(name.clone());
                        return;
                    }
                }
            }
            if matches!(expr, Expr::Arrow(_, _)) {
                return;
            }
            walk_expr_mut(self, expr);
        }
    }

    let mut scan = Scan {
        candidates: &candidates,
        in_closure: false,
        disqualified: HashSet::new(),
        slot_uses: HashSet::new(),
    };
    scan.visit_stmts(body);
    let Scan {
        disqualified,
        slot_uses,
        ..
    } = scan;
    let unbox: HashSet<String> = candidates
        .into_iter()
        .filter(|name| !disqualified.contains(name) && slot_uses.contains(name))
        .collect();
    if unbox.is_empty() {
        return;
    }
    Rewriter(&unbox).visit_stmts_mut(body);
    for (name, init) in declarations.iter_mut() {
        if unbox.contains(name) {
            *init = None;
        }
    }
}

/// Simplifies zero-offset index arithmetic (`arr[0 + idx]` / `arr[idx + 0]` -> `arr[idx]`) and
/// slot/slice literals (`{ buf: arr, off: 0 + idx [, len: n] }`) when `idx` is a proven non-string
/// numeric offset expression, and propagates dominated single-assignment `+0` local variables
/// (such as `v$buf = vec.buf, v$off = 0, v$len = vec.len; ... v$buf[v$off + i]`) to literal `0`.
///
/// Running this before `fold_offset_slot_derefs` and `cse_slice_loads` exposes `{ buf: arr, off: i }`
/// slot derefs and `arr[i].prop` repeated slice loads for zero-offset specialized slice parameters
/// (`off === 0`) and intra-procedural `Vec::as_slice` / `Vec::as_mut_slice` views.
pub(super) fn fold_zero_offset_arith(body: &mut [Stmt], uninit_locals: &HashSet<String>) {
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

    fn visit_expr(expr: &mut Expr, active_zeros: &HashSet<String>) {
        struct V<'a>(&'a HashSet<String>);
        impl VisitMut for V<'_> {
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
                        let is_slot_or_slice =
                            (entries.len() == 2 && entries[0].0 == "buf" && entries[1].0 == "off")
                                || (entries.len() == 3
                                    && entries[0].0 == "buf"
                                    && entries[1].0 == "off"
                                    && entries[2].0 == "len");
                        if is_slot_or_slice {
                            simplify_offset(&mut entries[1].1);
                        }
                    }
                    Expr::Arrow(_, ArrowBody::Expr(b)) => visit_expr(b, &HashSet::new()),
                    Expr::Arrow(_, ArrowBody::Block(stmts)) => {
                        let mut empty = HashSet::new();
                        visit_stmts(stmts, &HashSet::new(), &mut empty);
                    }
                    _ => walk_expr_mut(self, expr),
                }
            }
        }
        V(active_zeros).visit_expr_mut(expr);
    }

    fn visit_seq_stmt_expr(
        expr: &mut Expr,
        zero_candidates: &HashSet<String>,
        active_zeros: &mut HashSet<String>,
    ) {
        if let Expr::Seq(items) = expr {
            for item in items.iter_mut() {
                visit_seq_stmt_expr(item, zero_candidates, active_zeros);
            }
            return;
        }
        visit_expr(expr, active_zeros);
        if let Expr::Assign(lhs, rhs) = expr {
            if let Expr::Ident(name) = lhs.as_ref() {
                if zero_candidates.contains(name) && is_pos_zero(rhs) {
                    active_zeros.insert(name.clone());
                }
            }
        }
    }

    fn visit_stmts(
        stmts: &mut [Stmt],
        zero_candidates: &HashSet<String>,
        active_zeros: &mut HashSet<String>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::ExprStmt(e) => visit_seq_stmt_expr(e, zero_candidates, active_zeros),
                Stmt::Throw(e) | Stmt::Const(_, e) | Stmt::Return(Some(e)) => {
                    visit_expr(e, active_zeros);
                }
                Stmt::Let(name, Some(e)) => {
                    visit_expr(e, active_zeros);
                    if zero_candidates.contains(name) && is_pos_zero(e) {
                        active_zeros.insert(name.clone());
                    }
                }
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(e) = init {
                            visit_expr(e, active_zeros);
                            if zero_candidates.contains(name) && is_pos_zero(e) {
                                active_zeros.insert(name.clone());
                            }
                        }
                    }
                }
                Stmt::If(test, then_b, else_b) => {
                    visit_expr(test, active_zeros);
                    let mut then_z = active_zeros.clone();
                    visit_stmts(then_b, zero_candidates, &mut then_z);
                    if let Some(els) = else_b {
                        let mut else_z = active_zeros.clone();
                        visit_stmts(els, zero_candidates, &mut else_z);
                    }
                }
                Stmt::Switch(disc, cases) => {
                    visit_expr(disc, active_zeros);
                    for case in cases {
                        for t in &mut case.tests {
                            visit_expr(t, active_zeros);
                        }
                        let mut case_z = active_zeros.clone();
                        visit_stmts(&mut case.body, zero_candidates, &mut case_z);
                    }
                }
                Stmt::Labeled(_, inner) => {
                    let mut inner_z = active_zeros.clone();
                    visit_stmts(
                        std::slice::from_mut(inner.as_mut()),
                        zero_candidates,
                        &mut inner_z,
                    );
                }
                Stmt::LoopForever(_, b) | Stmt::Block(b) => {
                    let mut sub_z = active_zeros.clone();
                    visit_stmts(b, zero_candidates, &mut sub_z);
                }
                Stmt::FunctionDecl { body: b, .. } => {
                    let mut empty = HashSet::new();
                    visit_stmts(b, &HashSet::new(), &mut empty);
                }
                Stmt::While(test, b) => {
                    visit_expr(test, active_zeros);
                    let mut sub_z = active_zeros.clone();
                    visit_stmts(b, zero_candidates, &mut sub_z);
                }
                Stmt::TryCatch { body, handler, .. } => {
                    let mut body_z = active_zeros.clone();
                    visit_stmts(body, zero_candidates, &mut body_z);
                    let mut handler_z = active_zeros.clone();
                    visit_stmts(handler, zero_candidates, &mut handler_z);
                }
                _ => {}
            }
        }
    }

    let zero_candidates: HashSet<String> = if uninit_locals.is_empty() {
        HashSet::new()
    } else {
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
    };
    let mut active_zeros = HashSet::new();
    visit_stmts(body, &zero_candidates, &mut active_zeros);
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
        RawNum(String),
        Member(String, String),
    }

    impl SimpleIdx {
        fn to_expr(&self) -> Expr {
            match self {
                Self::Ident(s) => Expr::Ident(s.clone()),
                Self::Num(n) => Expr::Num(*n),
                Self::RawNum(s) => Expr::RawNum(s.clone()),
                Self::Member(obj, prop) => jsast::member(Expr::Ident(obj.clone()), prop.clone()),
            }
        }
        fn eq_idx(&self, other: &Self) -> bool {
            match (self, other) {
                (Self::Ident(a), Self::Ident(b)) => a == b,
                (Self::Num(a), Self::Num(b)) => a.to_bits() == b.to_bits(),
                (Self::RawNum(a), Self::RawNum(b)) => a == b,
                (Self::Member(o1, p1), Self::Member(o2, p2)) => o1 == o2 && p1 == p2,
                _ => false,
            }
        }
    }

    #[derive(Clone, PartialEq, Eq)]
    enum BaseRef {
        Ident(String),
        Member(String, String),
    }

    impl BaseRef {
        fn to_expr(&self) -> Expr {
            match self {
                Self::Ident(s) => Expr::Ident(s.clone()),
                Self::Member(obj, prop) => jsast::member(Expr::Ident(obj.clone()), prop.clone()),
            }
        }

        fn root_ident(&self) -> &str {
            match self {
                Self::Ident(s) | Self::Member(s, _) => s.as_str(),
            }
        }
    }

    #[derive(Clone)]
    enum SlotDep {
        Ident(String),
        /// Member chain `obj.prop` (sensitive to `obj = ...`, `obj.prop = ...`, `obj.prop.{buf,off} = ...`,
        /// but NOT to sibling writes like `obj.pos = ...` or `obj.end = ...`).
        Member(String, String),
    }

    #[derive(Clone)]
    enum SlotDef {
        Offset {
            base: BaseRef,
            op: jsast::BinOp,
            idx: SimpleIdx,
        },
        Direct {
            buf: BaseRef,
            off: SimpleIdx,
        },
    }

    impl SlotDef {
        fn replacement(&self) -> Expr {
            match self {
                Self::Offset { base, op, idx } => jsast::index(
                    jsast::member(base.to_expr(), "buf"),
                    jsast::binary(*op, jsast::member(base.to_expr(), "off"), idx.to_expr()),
                ),
                Self::Direct { buf, off } => jsast::index(buf.to_expr(), off.to_expr()),
            }
        }

        fn deps(&self) -> Vec<SlotDep> {
            let mut out = Vec::with_capacity(2);
            let push_base = |out: &mut Vec<SlotDep>, b: &BaseRef| match b {
                BaseRef::Ident(s) => out.push(SlotDep::Ident(s.clone())),
                BaseRef::Member(o, p) => out.push(SlotDep::Member(o.clone(), p.clone())),
            };
            let push_idx = |out: &mut Vec<SlotDep>, idx: &SimpleIdx| match idx {
                SimpleIdx::Ident(i) => out.push(SlotDep::Ident(i.clone())),
                SimpleIdx::Member(o, p) => out.push(SlotDep::Member(o.clone(), p.clone())),
                SimpleIdx::Num(_) | SimpleIdx::RawNum(_) => {}
            };
            match self {
                Self::Offset { base, idx, .. } => {
                    push_base(&mut out, base);
                    push_idx(&mut out, idx);
                }
                Self::Direct { buf, off } => {
                    push_base(&mut out, buf);
                    push_idx(&mut out, off);
                }
            }
            out
        }

        fn mentions_ident(&self, target: &str) -> bool {
            let idx_mentions = |idx: &SimpleIdx| match idx {
                SimpleIdx::Ident(s) | SimpleIdx::Member(s, _) => s == target,
                _ => false,
            };
            match self {
                Self::Offset { base, idx, .. } => base.root_ident() == target || idx_mentions(idx),
                Self::Direct { buf, off } => buf.root_ident() == target || idx_mentions(off),
            }
        }

        fn is_offset(&self) -> bool {
            matches!(self, Self::Offset { .. })
        }
    }

    fn match_base_ref(expr: &Expr) -> Option<BaseRef> {
        match expr {
            Expr::Ident(s) => Some(BaseRef::Ident(s.clone())),
            Expr::Member(obj, prop) => match obj.as_ref() {
                Expr::Ident(s) => Some(BaseRef::Member(s.clone(), prop.clone())),
                _ => None,
            },
            _ => None,
        }
    }

    fn match_simple_idx(expr: &Expr) -> Option<SimpleIdx> {
        match expr {
            Expr::Ident(s) => Some(SimpleIdx::Ident(s.clone())),
            Expr::Num(n) => Some(SimpleIdx::Num(*n)),
            Expr::RawNum(s) => Some(SimpleIdx::RawNum(s.clone())),
            Expr::Member(obj, prop) => match obj.as_ref() {
                Expr::Ident(s) => Some(SimpleIdx::Member(s.clone(), prop.clone())),
                _ => None,
            },
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

    fn match_offset_obj(expr: &Expr) -> Option<(BaseRef, jsast::BinOp, SimpleIdx)> {
        let Expr::Object(entries) = expr else {
            return None;
        };
        if entries.len() != 2 || entries[0].0 != "buf" || entries[1].0 != "off" {
            return None;
        }
        let Expr::Member(b1, bp) = &entries[0].1 else {
            return None;
        };
        if bp != "buf" {
            return None;
        }
        let base1 = match_base_ref(b1)?;
        let Expr::Binary(op @ (jsast::BinOp::Add | jsast::BinOp::Sub), lhs, rhs) = &entries[1].1
        else {
            return None;
        };
        let Expr::Member(b2, op_prop) = lhs.as_ref() else {
            return None;
        };
        if op_prop != "off" {
            return None;
        }
        let base2 = match_base_ref(b2)?;
        if base1 != base2 {
            return None;
        }
        let idx = match_simple_idx(rhs)?;
        Some((base1, *op, idx))
    }

    fn match_slot_def(target: &str, rhs: &Expr) -> Option<SlotDef> {
        // Case 1: `idx === 0 ? base : { buf: base.buf, off: base.off +/- idx }`
        if let Expr::Cond(cond, then_e, else_e) = rhs {
            if let (
                Expr::Binary(jsast::BinOp::StrictEq, c_lhs, c_rhs),
                Some(then_base),
                Some((else_base, op, else_idx)),
            ) = (
                cond.as_ref(),
                match_base_ref(then_e),
                match_offset_obj(else_e),
            ) {
                let c_zero = matches!(c_rhs.as_ref(), Expr::Num(n) if *n == 0.0);
                if c_zero
                    && then_base == else_base
                    && match_simple_idx(c_lhs).is_some_and(|ci| ci.eq_idx(&else_idx))
                {
                    let def = SlotDef::Offset {
                        base: else_base,
                        op,
                        idx: else_idx,
                    };
                    if !def.mentions_ident(target) {
                        return Some(def);
                    }
                }
            }
            return None;
        }
        // Case 2: `{ buf: base.buf, off: base.off +/- idx }`
        if let Some((base, op, idx)) = match_offset_obj(rhs) {
            let def = SlotDef::Offset { base, op, idx };
            if !def.mentions_ident(target) {
                return Some(def);
            }
            return None;
        }
        // Case 3: `{ buf: arr, off: k }`
        if let Expr::Object(entries) = rhs {
            if entries.len() == 2 && entries[0].0 == "buf" && entries[1].0 == "off" {
                if let (Some(buf), Some(off)) = (
                    match_base_ref(&entries[0].1),
                    match_simple_idx(&entries[1].1),
                ) {
                    let def = SlotDef::Direct { buf, off };
                    if !def.mentions_ident(target) {
                        return Some(def);
                    }
                }
            }
        }
        None
    }

    struct SlotScan<'a> {
        locals: &'a HashSet<String>,
        in_closure: bool,
        usage: HashMap<String, SlotUsage>,
    }
    impl Visit for SlotScan<'_> {
        fn visit_stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                    if !self.in_closure
                        && matches!(lhs.as_ref(), Expr::Ident(n) if self.locals.contains(n)) =>
                {
                    let Expr::Ident(name) = lhs.as_ref() else {
                        unreachable!()
                    };
                    self.usage.entry(name.clone()).or_default().writes += 1;
                    self.visit_expr(rhs);
                }
                Stmt::Let(name, Some(init)) if !self.in_closure && self.locals.contains(name) => {
                    self.usage.entry(name.clone()).or_default().writes += 1;
                    self.visit_expr(init);
                }
                Stmt::Lets(decls) => {
                    for (name, init) in decls {
                        if let Some(init) = init {
                            if !self.in_closure && self.locals.contains(name) {
                                self.usage.entry(name.clone()).or_default().writes += 1;
                            }
                            self.visit_expr(init);
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
            if !self.in_closure {
                if let Some(name) = match_slot_deref(expr) {
                    if self.locals.contains(name) {
                        self.usage.entry(name.to_owned()).or_default().slot_derefs += 1;
                        return;
                    }
                }
            }
            match expr {
                Expr::Ident(name) if self.locals.contains(name) => {
                    self.usage.entry(name.clone()).or_default().other_uses += 1;
                }
                Expr::Assign(lhs, rhs) => {
                    if !self.in_closure {
                        if let Some(name) = match_slot_deref(lhs) {
                            if self.locals.contains(name) {
                                self.usage.entry(name.to_owned()).or_default().slot_derefs += 1;
                                self.visit_expr(rhs);
                                return;
                            }
                        }
                    }
                    self.visit_expr(lhs);
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

    fn count_derefs_in_stmt(stmt: &Stmt, target: &str) -> usize {
        struct V<'a> {
            target: &'a str,
            count: usize,
        }
        impl Visit for V<'_> {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                if matches!(stmt, Stmt::FunctionDecl { .. }) {
                    return;
                }
                walk_stmt(self, stmt);
            }
            fn visit_expr(&mut self, expr: &Expr) {
                if match_slot_deref(expr) == Some(self.target) {
                    self.count += 1;
                    return;
                }
                if matches!(expr, Expr::Arrow(_, _)) {
                    return;
                }
                walk_expr(self, expr);
            }
        }
        let mut v = V { target, count: 0 };
        v.visit_stmt(stmt);
        v.count
    }

    fn dep_matches_ident(deps: &[SlotDep], name: &str) -> bool {
        deps.iter().any(|d| match d {
            SlotDep::Ident(s) | SlotDep::Member(s, _) => s == name,
        })
    }

    fn lhs_writes_dep(lhs: &Expr, deps: &[SlotDep]) -> bool {
        match lhs {
            Expr::Ident(n) => dep_matches_ident(deps, n),
            Expr::Member(obj, prop) => match obj.as_ref() {
                Expr::Ident(n) => deps.iter().any(|d| match d {
                    SlotDep::Ident(s) => s == n && (prop == "buf" || prop == "off"),
                    SlotDep::Member(s, p) => s == n && p == prop,
                }),
                Expr::Member(inner, inner_prop) if prop == "buf" || prop == "off" => {
                    matches!(inner.as_ref(), Expr::Ident(n) if deps.iter().any(|d| matches!(d, SlotDep::Member(s, p) if s == n && p == inner_prop)))
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn stmt_mutates_deps_or_calls(stmt: &Stmt, deps: &[SlotDep], forbid_calls: bool) -> bool {
        struct V<'a> {
            deps: &'a [SlotDep],
            forbid_calls: bool,
            found: bool,
        }
        impl Visit for V<'_> {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                if self.found {
                    return;
                }
                match stmt {
                    Stmt::Let(n, _) | Stmt::Const(n, _) if dep_matches_ident(self.deps, n) => {
                        self.found = true;
                        return;
                    }
                    Stmt::Lets(decls)
                        if decls.iter().any(|(n, _)| dep_matches_ident(self.deps, n)) =>
                    {
                        self.found = true;
                        return;
                    }
                    Stmt::FunctionDecl { .. } => return,
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
                        if lhs_writes_dep(lhs, self.deps) =>
                    {
                        self.found = true;
                    }
                    Expr::Call(c, args) => {
                        if matches!(c.as_ref(), Expr::Ident(n) if n.starts_with("core$panicking$"))
                        {
                            let prev = self.forbid_calls;
                            self.forbid_calls = false;
                            for a in args {
                                self.visit_expr(a);
                            }
                            self.forbid_calls = prev;
                            return;
                        }
                        if self.forbid_calls {
                            self.found = true;
                            return;
                        }
                        walk_expr(self, expr);
                    }
                    Expr::New(_, _) => {
                        if self.forbid_calls {
                            self.found = true;
                            return;
                        }
                        walk_expr(self, expr);
                    }
                    Expr::Arrow(_, _) => {}
                    _ => walk_expr(self, expr),
                }
            }
        }
        let mut v = V {
            deps,
            forbid_calls,
            found: false,
        };
        v.visit_stmt(stmt);
        v.found
    }

    fn replace_deref_in_stmt(stmt: &mut Stmt, target: &str, replacement: &Expr) {
        struct V<'a> {
            target: &'a str,
            replacement: &'a Expr,
        }
        impl VisitMut for V<'_> {
            fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
                if matches!(stmt, Stmt::FunctionDecl { .. }) {
                    return;
                }
                walk_stmt_mut(self, stmt);
            }
            fn visit_expr_mut(&mut self, expr: &mut Expr) {
                if match_slot_deref(expr) == Some(self.target) {
                    *expr = self.replacement.clone();
                    return;
                }
                if matches!(expr, Expr::Arrow(_, _)) {
                    return;
                }
                walk_expr_mut(self, expr);
            }
        }
        V {
            target,
            replacement,
        }
        .visit_stmt_mut(stmt);
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

    let mut scan = SlotScan {
        locals,
        in_closure: false,
        usage: HashMap::new(),
    };
    scan.visit_stmts(body);
    let eligible: HashMap<String, usize> = scan
        .usage
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
        struct V<'a> {
            target: &'a str,
            found: bool,
        }
        impl Visit for V<'_> {
            fn visit_expr(&mut self, expr: &Expr) {
                if self.found {
                    return;
                }
                match expr {
                    Expr::Ident(n) if n == self.target => self.found = true,
                    Expr::Arrow(_, _) => {}
                    _ => walk_expr(self, expr),
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

    fn scan_sfp_usage(stmts: &[Stmt], locals: &HashSet<String>) -> HashMap<String, SfpUsage> {
        struct Scan<'a> {
            locals: &'a HashSet<String>,
            in_closure: bool,
            usage: HashMap<String, SfpUsage>,
        }
        impl Visit for Scan<'_> {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                match stmt {
                    Stmt::ExprStmt(Expr::Assign(lhs, rhs))
                        if !self.in_closure
                            && matches!(lhs.as_ref(), Expr::Ident(n) if self.locals.contains(n)) =>
                    {
                        let Expr::Ident(name) = lhs.as_ref() else {
                            unreachable!()
                        };
                        self.usage.entry(name.clone()).or_default().writes += 1;
                        self.visit_expr(rhs);
                    }
                    Stmt::Let(name, Some(init))
                        if !self.in_closure && self.locals.contains(name) =>
                    {
                        self.usage.entry(name.clone()).or_default().writes += 1;
                        self.visit_expr(init);
                    }
                    Stmt::Lets(decls) => {
                        for (name, init) in decls {
                            if let Some(init) = init {
                                if !self.in_closure && self.locals.contains(name) {
                                    self.usage.entry(name.clone()).or_default().writes += 1;
                                }
                                self.visit_expr(init);
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
                if !self.in_closure {
                    if let Expr::Index(obj, idx) = expr {
                        if let Some(name) = is_v_member(obj, "buf") {
                            if self.locals.contains(name) && is_v_off_index(idx, name) {
                                self.usage.entry(name.to_owned()).or_default().index_uses += 1;
                                if let Expr::Binary(_, _, r) = idx.as_ref() {
                                    self.visit_expr(r);
                                }
                                return;
                            }
                        }
                    }
                    if let Some(name) = is_v_member(expr, "len") {
                        if self.locals.contains(name) {
                            self.usage.entry(name.to_owned()).or_default().len_reads += 1;
                            return;
                        }
                    }
                }
                match expr {
                    Expr::Ident(n) if self.locals.contains(n) => {
                        self.usage.entry(n.clone()).or_default().other_uses += 1;
                    }
                    Expr::Assign(lhs, rhs) => {
                        if !self.in_closure {
                            if let Expr::Index(obj, idx) = lhs.as_ref() {
                                if let Some(name) = is_v_member(obj, "buf") {
                                    if self.locals.contains(name) && is_v_off_index(idx, name) {
                                        self.usage
                                            .entry(name.to_owned())
                                            .or_default()
                                            .index_uses += 1;
                                        if let Expr::Binary(_, _, r) = idx.as_ref() {
                                            self.visit_expr(r);
                                        }
                                        self.visit_expr(rhs);
                                        return;
                                    }
                                }
                            }
                        }
                        self.visit_expr(lhs);
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
        let mut scan = Scan {
            locals,
            in_closure: false,
            usage: HashMap::new(),
        };
        scan.visit_stmts(stmts);
        scan.usage
    }

    fn is_pure_len_expr(expr: &Expr) -> bool {
        match expr {
            Expr::Ident(_) | Expr::Num(_) | Expr::RawNum(_) => true,
            Expr::Member(obj, _) => is_pure_len_expr(obj),
            _ => false,
        }
    }

    fn rewrite_stmts(stmts: &mut [Stmt], deref_only: &HashSet<String>) {
        struct V<'a>(&'a HashSet<String>);
        impl VisitMut for V<'_> {
            fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
                match stmt {
                    Stmt::ExprStmt(Expr::Assign(lhs, rhs)) if matches!(lhs.as_ref(), Expr::Ident(n) if self.0.contains(n)) => {
                        if let Some((ptr_arg, len_arg)) = match_sfp_call(rhs) {
                            if jsast::is_droppable(len_arg) || is_pure_len_expr(len_arg) {
                                **rhs = ptr_arg.clone();
                            }
                        }
                    }
                    Stmt::Let(name, Some(init)) if self.0.contains(name) => {
                        if let Some((ptr_arg, len_arg)) = match_sfp_call(init) {
                            if jsast::is_droppable(len_arg) || is_pure_len_expr(len_arg) {
                                *init = ptr_arg.clone();
                            }
                        }
                    }
                    Stmt::FunctionDecl { .. } => {}
                    _ => walk_stmt_mut(self, stmt),
                }
            }
            fn visit_expr_mut(&mut self, _: &mut Expr) {}
        }
        V(deref_only).visit_stmts_mut(stmts);
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
            struct V<'a> {
                var: &'a str,
                replacement: &'a Expr,
                count: usize,
            }
            impl VisitMut for V<'_> {
                fn visit_expr_mut(&mut self, expr: &mut Expr) {
                    if is_v_member(expr, "len") == Some(self.var) {
                        *expr = self.replacement.clone();
                        self.count += 1;
                        return;
                    }
                    if matches!(expr, Expr::Arrow(_, _)) {
                        return;
                    }
                    walk_expr_mut(self, expr);
                }
            }
            let mut v = V {
                var,
                replacement,
                count: 0,
            };
            v.visit_expr_mut(expr);
            v.count
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
                                    super::ast_util::rename_ident_stmts(
                                        &mut stmts[i + 1..],
                                        &from,
                                        &to,
                                    );
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

        let usage = scan_sfp_usage(body, locals);
        let sfp_vars: HashSet<String> = usage
            .iter()
            .filter(|(name, u)| u.writes == 1 && !super::ast_util::has_closure_use(body, name))
            .map(|(name, _)| name.clone())
            .collect();
        let single_write_aliases: HashSet<String> = usage
            .iter()
            .filter(|(name, u)| {
                u.writes == 1
                    && (u.index_uses > 0 || u.len_reads > 0)
                    && u.other_uses == 0
                    && !super::ast_util::has_closure_use(body, name)
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

    let mut usage = scan_sfp_usage(body, locals);
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
            usage = scan_sfp_usage(body, locals);
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
        struct V(bool);
        impl Visit for V {
            fn visit_expr(&mut self, expr: &Expr) {
                if self.0 {
                    return;
                }
                match expr {
                    Expr::Call(_, _) | Expr::New(_, _) => self.0 = true,
                    Expr::Arrow(_, _) => {}
                    _ => walk_expr(self, expr),
                }
            }
        }
        let mut v = V(false);
        v.visit_expr(expr);
        v.0
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

    fn stmt_invalidates(stmt: &Stmt, key: &SliceLoadKey) -> bool {
        struct V<'a> {
            key: &'a SliceLoadKey,
            found: bool,
        }
        impl Visit for V<'_> {
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
        let mut v = V { key, found: false };
        v.visit_stmt(stmt);
        v.found
    }

    fn count_in_stmt(stmt: &Stmt, key: &SliceLoadKey) -> usize {
        struct V<'a> {
            key: &'a SliceLoadKey,
            count: usize,
        }
        impl Visit for V<'_> {
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
        let mut v = V { key, count: 0 };
        v.visit_stmt(stmt);
        v.count
    }

    fn replace_in_stmt(stmt: &mut Stmt, key: &SliceLoadKey, var: &str) {
        struct V<'a> {
            key: &'a SliceLoadKey,
            var: &'a str,
        }
        impl VisitMut for V<'_> {
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
        V { key, var }.visit_stmt_mut(stmt);
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

    #[test]
    fn dead_pure_ternary_and_single_slot_array_cells_unbox() {
        let mut declarations = vec![
            ("m".to_owned(), Some(jsast::array(Vec::new()))),
            ("k".to_owned(), Some(jsast::array(Vec::new()))),
            ("esc".to_owned(), Some(jsast::array(Vec::new()))),
            ("o".to_owned(), None),
            ("p".to_owned(), None),
            ("q".to_owned(), None),
            ("r".to_owned(), None),
            ("s".to_owned(), None),
        ];
        let local_names: HashSet<String> = declarations.iter().map(|(n, _)| n.clone()).collect();
        let mut body = vec![
            jsast::assign_stmt(
                jsast::index(jsast::id("m"), jsast::num(0.0)),
                jsast::id("b"),
            ),
            jsast::assign_stmt(
                jsast::index(jsast::id("k"), jsast::num(0.0)),
                jsast::id("e"),
            ),
            jsast::assign_stmt(
                jsast::id("o"),
                jsast::object_of(vec![("buf", jsast::id("m")), ("off", jsast::num(0.0))]),
            ),
            jsast::assign_stmt(
                jsast::id("p"),
                jsast::object_of(vec![("buf", jsast::id("k")), ("off", jsast::num(0.0))]),
            ),
            Stmt::ExprStmt(jsast::cond(jsast::id("j"), jsast::id("o"), jsast::id("p"))),
            jsast::assign_stmt(
                jsast::id("q"),
                jsast::index(jsast::id("m"), jsast::num(0.0)),
            ),
            jsast::assign_stmt(
                jsast::id("r"),
                jsast::index(jsast::id("k"), jsast::num(0.0)),
            ),
            jsast::assign_stmt(
                jsast::id("s"),
                jsast::cond(jsast::id("j"), jsast::id("q"), jsast::id("r")),
            ),
            jsast::assign_stmt(
                jsast::index(jsast::id("esc"), jsast::num(0.0)),
                jsast::id("s"),
            ),
            Stmt::Return(Some(jsast::object_of(vec![
                ("buf", jsast::id("esc")),
                ("off", jsast::num(0.0)),
            ]))),
        ];
        eliminate_dead_local_writes(&mut body, &local_names);
        unbox_single_slot_array_locals(&mut body, &mut declarations);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(!printed.contains("j ? o : p"), "{printed}");
        assert!(!printed.contains("m[0]"), "{printed}");
        assert!(!printed.contains("k[0]"), "{printed}");
        assert!(printed.contains("m = b;"), "{printed}");
        assert!(printed.contains("esc[0] = s;"), "{printed}");
        assert_eq!(declarations[0], ("m".to_owned(), None));
        assert_eq!(declarations[1], ("k".to_owned(), None));
        assert!(declarations[2].1.is_some(), "escaping cell must stay boxed");
    }

    #[test]
    fn collapse_ephemeral_rt_box_deref_elides_temp_box() {
        let locals: HashSet<String> = ["f".to_owned(), "g".to_owned(), "kept".to_owned()]
            .into_iter()
            .collect();
        let ephemeral = Expr::Seq(vec![
            Expr::Assign(
                Box::new(jsast::id("f")),
                Box::new(jsast::rt_call(
                    "box",
                    vec![jsast::member(jsast::id("obj"), "value")],
                )),
            ),
            Expr::Index(
                Box::new(jsast::member(jsast::id("f"), "buf")),
                Box::new(jsast::member(jsast::id("f"), "off")),
            ),
        ]);
        let retained = Expr::Seq(vec![
            Expr::Assign(
                Box::new(jsast::id("kept")),
                Box::new(jsast::rt_call("box", vec![jsast::id("other")])),
            ),
            Expr::Index(
                Box::new(jsast::member(jsast::id("kept"), "buf")),
                Box::new(jsast::member(jsast::id("kept"), "off")),
            ),
        ]);
        let mut body = vec![
            jsast::assign_stmt(jsast::id("g"), ephemeral),
            jsast::assign_stmt(jsast::id("h"), retained),
            Stmt::Return(Some(jsast::id("kept"))),
        ];
        collapse_ephemeral_rt_box_deref(&mut body, &locals);
        let printed = jsast::stmt_to_string(&Stmt::Block(body));
        assert!(printed.contains("g = obj.value;"), "{printed}");
        assert!(
            printed.contains("__rt.box(other)"),
            "multi-use boxed slot must stay intact:\n{printed}"
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
            // `p = { buf: a, off: 0 + i };` -> `p = { buf: a, off: i };`
            jsast::assign_stmt(
                jsast::id("p"),
                jsast::object_of(vec![
                    ("buf", jsast::id("a")),
                    (
                        "off",
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
        assert!(printed.contains("p = { buf: a, off: i };"), "{printed}");
        assert!(printed.contains("z = 0 + s;"), "{printed}");
    }
}
