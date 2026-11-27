//! One item as a top level declaration: a function (`base.rs::codegen_fn` and `codegen_body`),
//! a `static` (`base.rs::codegen_static`), or a vtable (`vtable.rs`).

use std::collections::{BTreeSet, HashSet};

use rcgjs_object::item::JsName;
use rcgjs_object::jir::decline::{Decline, OpClass};
use rcgjs_object::jir::repr::Func;
use rcgjs_object::jir::semantic::{BodyKind, BodySemantic, VtableEntry};
use rcgjs_object::jsast::{self, Expr, Stmt};
use rcgjs_object::ptr;
use structurizer::Region;

use crate::opts::OptimizerPass;

use super::super::plan::{BlockPayload, Plan, RScrut};
use super::super::{LowerInput, LoweredItem};
use super::{decline, emit, fn_opt, value, JsCx, ProgramPlans};

pub(crate) use super::fn_opt::fold_bstr_in_bytes_decl;
#[allow(
    unused_imports,
    reason = "re-exported for crate-internal and test callers"
)]
pub(crate) use super::fn_opt::fold_bstr_in_bytes_stmts;
#[cfg(test)]
pub(super) use super::fn_opt::fuse_local_bytes_str;
#[allow(
    unused_imports,
    reason = "re-exported for sibling lowering modules and tests"
)]
pub(super) use super::fn_opt::{
    cse_slice_loads, fold_offset_slot_derefs, fold_sfp_deref_only, fold_zero_offset_arith,
    fuse_local_bytes_str_with,
};

struct LoweredParams {
    params: Vec<String>,
    formal_param_names: HashSet<String>,
    param_names: HashSet<String>,
    unpacked_i64_params: Vec<(String, String, String)>,
}

struct LoweredBody {
    body: Vec<Stmt>,
    declarations: Vec<(String, Option<Expr>)>,
    local_names: HashSet<String>,
}

/// Lowers a function body whose SSA form, plan and region tree are in hand.
#[allow(
    dead_code,
    reason = "public lowering entrypoint used by unit tests and probes"
)]
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
    lower_with_plans(input, func, plan, region, ascii, None)
}

/// [`lower`] carrying whole-program ASCII provenance facts and SoA `Vec` plan.
pub fn lower_with_plans(
    input: &LowerInput<'_>,
    func: &Func,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
    ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'_>>,
    soa: Option<&crate::plan::soa_vec::SoaPlan>,
) -> Result<LoweredItem, Decline> {
    let physical = input
        .abi
        .and_then(|abi| abi.target_signature(input.name, func.item));
    if let Some(lowered) = try_lower_noop_drop_glue(input, func) {
        return Ok(lowered);
    }
    if input.body.semantic == BodySemantic::JsExportOut {
        return lower_js_export_out(input, func);
    }
    if input.body.semantic
        == BodySemantic::Standard(rcgjs_object::jir::semantic::StandardOperation::SliceSort)
    {
        if let Some(lowered) = try_lower_slice_sort(input, func, physical)? {
            return Ok(lowered);
        }
    }
    let cx = build_js_cx(input, func, plan, physical, ascii, soa);
    let mut lowered_params = setup_flat_abi_params(&cx, input, func, plan, physical)?;
    let mut lowered_body = lower_body_regions(
        &cx,
        plan,
        region,
        &lowered_params.formal_param_names,
        &lowered_params.param_names,
        std::mem::take(&mut lowered_params.unpacked_i64_params),
    )?;
    apply_local_post_passes(&cx, input, &mut lowered_body);
    build_function_decl(&cx, input, func, plan, lowered_params, lowered_body)
}

fn try_lower_noop_drop_glue(input: &LowerInput<'_>, func: &Func) -> Option<LoweredItem> {
    let heap = input.heap?;
    let BodySemantic::DropGlue(ty) = input.body.semantic else {
        return None;
    };
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
    if !is_noop_str && !is_noop_vec {
        return None;
    }
    let params: Vec<String> = (0..func.params.len()).map(|i| format!("$p{i}")).collect();
    let decl = jsast::function(input.name.as_str(), params, Vec::new());
    Some(LoweredItem {
        decl,
        refs: BTreeSet::new(),
        zombies: Vec::new(),
        loc: None,
        names: Vec::new(),
        dts: None,
        js_alloc_origins: Vec::new(),
        bigints: BTreeSet::new(),
    })
}

/// Directly lowers an internal `BodySemantic::JsExportOut` helper (`link_section =
/// "rcgjs.export.out.v1"`) emitted by `#[js_export] async fn`, replacing its dummy MIR body with
/// Rust→JS return marshaling (and `Result` unwrap/throw) before any MIR lowering runs.
fn lower_js_export_out(input: &LowerInput<'_>, func: &Func) -> Result<LoweredItem, Decline> {
    let Some(&value_ty) = func.sig.arguments.first() else {
        return Err(decline(
            OpClass::Other,
            "BodySemantic::JsExportOut function must take one argument",
        ));
    };
    let default_heap = crate::analysis::heap::HeapPlan::default();
    let boundary = super::export_boundary::BoundaryCx {
        table: input.table,
        interns: input.interns,
        heap: input.heap.unwrap_or(&default_heap),
        bstr: crate::lower::bstr::enabled_for(input.opts),
    };
    let param = "$p0".to_owned();
    let body = super::export_boundary::lower_export_out_body(&boundary, &param, value_ty);
    let decl = jsast::function(input.name.as_str(), vec![param], body);
    finish(
        input,
        decl,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        input.bigints,
        &HashSet::new(),
    )
}

fn build_js_cx<'a>(
    input: &LowerInput<'a>,
    func: &'a Func,
    plan: &'a Plan,
    physical: Option<&'a crate::plan::abi_flat::FlatSig>,
    ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'a>>,
    soa: Option<&'a crate::plan::soa_vec::SoaPlan>,
) -> JsCx<'a> {
    JsCx::with_plans(
        func,
        plan,
        input.table,
        input.interns,
        input.opts,
        input.summary,
        ProgramPlans {
            origins: input.origins,
            items: input.items,
            drop_glue: input.drop_glue,
            vtables: input.vtables,
            spills: input.spills,
            abi: input.abi,
            target_abi: physical,
            call_retargets: input.call_retargets,
            hoists_bigints: input.bigints.is_some(),
            byte_views: input.byte_views,
            heap: input.heap,
            soa,
            nested_clone: input.nested_clone,
            ascii,
        },
    )
}

/// Binds physical slice/slot locals and expands flat direct-call ABI parameters (`base.rs::params`),
/// then verifies every `ByteStrParam` parameter survived `byte_view_analysis`.
fn setup_flat_abi_params(
    cx: &JsCx<'_>,
    input: &LowerInput<'_>,
    func: &Func,
    plan: &Plan,
    physical: Option<&crate::plan::abi_flat::FlatSig>,
) -> Result<LoweredParams, Decline> {
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

    Ok(LoweredParams {
        params,
        formal_param_names,
        param_names,
        unpacked_i64_params,
    })
}

/// Emits the structured region tree, prepends lazy `i64` entry unpacks, and builds the prelude
/// declaration list (`base.rs::codegen_body`).
fn lower_body_regions(
    cx: &JsCx<'_>,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
    formal_param_names: &HashSet<String>,
    param_names: &HashSet<String>,
    unpacked_i64_params: Vec<(String, String, String)>,
) -> Result<LoweredBody, Decline> {
    let (mut body, structure_temps) = emit::emit(cx, region)?;
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

    let mut declarations: Vec<(String, Option<Expr>)> = Vec::new();
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
    let mut local_names = declared;
    local_names.extend(param_names.iter().cloned());

    Ok(LoweredBody {
        body,
        declarations,
        local_names,
    })
}

fn apply_local_post_passes(cx: &JsCx<'_>, input: &LowerInput<'_>, lowered_body: &mut LoweredBody) {
    fn_opt::run_local_passes(
        &mut lowered_body.body,
        &mut lowered_body.declarations,
        &mut lowered_body.local_names,
        fn_opt::LocalOptConfig {
            ast_dce: input.opts.applies(OptimizerPass::AstDce),
            needs_byte_slice_ref: cx.needs_byte_slice_ref(),
            bstr_enabled: cx.bstr_enabled(),
            cse_slice_loads: input.opts.profile == Some(crate::opts::Profile::ReleaseSpeed)
                && !cx.byte_views.raw_byte_derefs
                && cx.byte_views.elide_scale_scalar,
        },
    );
    let mentioned = mentioned_names(&lowered_body.body);
    if input.opts.queue {
        let scoped = declared_names(&lowered_body.body);
        lowered_body
            .declarations
            .retain(|(name, _)| mentioned.contains(name) && !scoped.contains(name));
    }
}

fn build_function_decl(
    cx: &JsCx<'_>,
    input: &LowerInput<'_>,
    func: &Func,
    plan: &Plan,
    lowered_params: LoweredParams,
    lowered_body: LoweredBody,
) -> Result<LoweredItem, Decline> {
    let LoweredParams {
        params,
        formal_param_names,
        ..
    } = lowered_params;
    let LoweredBody {
        body, declarations, ..
    } = lowered_body;
    let mut out = Vec::new();
    if !declarations.is_empty() {
        out.push(jsast::lets(declarations));
    }
    out.extend(body);

    if input.body.semantic == BodySemantic::PanicImpl {
        out = jsast::wrap_panic_impl_body(out);
    }

    let mut marshaled_params = HashSet::new();
    if input.body.semantic == BodySemantic::JsExport && cx.target_abi.is_none() {
        let boundary = super::export_boundary::BoundaryCx {
            table: input.table,
            interns: input.interns,
            heap: cx.heap(),
            bstr: cx.bstr_enabled(),
        };
        marshaled_params =
            super::export_boundary::wrap_js_export(&boundary, func, &params, &mut out);
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
        if !is_boxed_param && formal_param_names.contains(name) && !marshaled_params.contains(name)
        {
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
    let cx = JsCx::with_plans(
        &func,
        plan,
        input.table,
        input.interns,
        input.opts,
        input.summary,
        ProgramPlans {
            origins: input.origins,
            items: input.items,
            drop_glue: input.drop_glue,
            vtables: input.vtables,
            spills: input.spills,
            abi: input.abi,
            call_retargets: input.call_retargets,
            ..ProgramPlans::default()
        },
    );
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
            fn_opt::run_post_sroa_passes(params, body);
        }
        if input.opts.profile == Some(crate::opts::Profile::ReleaseSpeed) {
            super::post_opt::run_speed_post_opts_with_mode(
                &mut decl,
                input.final_lowering,
                immutable_slices,
            );
            if let Stmt::FunctionDecl { params, body, .. } = &mut decl {
                fn_opt::run_post_speed_cleanup(params, body);
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
