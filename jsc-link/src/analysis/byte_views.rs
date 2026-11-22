//! Whole-program wide-byte-view reachability analysis (`MIRES-008` lazy-proxy recovery).
//!
//! Computes two conservative whole-program facts over reachable functions:
//!
//! 1. [`ByteViewPlan::raw_byte_derefs`]: whether a program can ever reach a byte dereference or
//!    byte memory op through a pre-retype plain `raw` allocation array (`block.typed !== undefined`,
//!    `raw.length === 0`) or an escaping non-scalar scaled pointer (`p.sc !== undefined`).
//!    When `false`, the JIR lowerer keeps every 1-byte dereference inline (`p.buf[p.off]`) and
//!    omits `__rt._bv_enable`.
//! 2. [`ByteViewPlan::elide_scale_scalar`]: when `raw_byte_derefs` is `false` and every wider
//!    scalar -> `u8`/`i8` cast (`scale_scalar`) in reachable functions only flows into stdlib
//!    container plumbing (`Vec`/`RawVec`/`Box`/`IntoIter`/`dealloc`/`realloc`/`unscale_scalar`)
//!    or integer/char equality checks (`compare_bytes(a, b, n) == 0` / `!= 0`), the JIR lowerer
//!    can emit Proxy-free `__rt.scale(p, size)` instead of `__rt.scale_scalar(p, size, kind)`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rcgjs_object::jir::decline::Decline;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{CellId, Func, Op, OpKind, Proj, RBlockId, RTerm, Root, ValueId};
use rcgjs_object::jir::semantic::{
    BinaryOp, Callee, CastKind, ConstTree, GlobalReference, UnaryOp,
};
use rcgjs_object::jir::types::{TypeId, TypeKind, TypeTable};
use rcgjs_object::jsast;

use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::plan::representation::NativeBody;

/// Whole-program lowering policy for lazy byte views.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ByteViewPlan {
    /// Emit `__rt.read_byte*`/`__rt.write_byte*` on 1-byte dereferences and retain
    /// `__rt._bv_enable` in the emitted program.
    pub raw_byte_derefs: bool,
    /// Replace non-escaping `scale_scalar` casts with Proxy-free `__rt.scale`.
    pub elide_scale_scalar: bool,
}

/// Computes the conservative whole-program [`ByteViewPlan`] over items reachable from program roots.
pub fn analyze_reachable(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    extra_roots: &[String],
) -> ByteViewPlan {
    let reachable_items = collect_reachable_items(table, funcs, native_bodies, extra_roots);
    let debug = std::env::var_os("RCGJS_BV_DEBUG").is_some();
    let mut raw_byte_derefs = false;
    let mut scalar_byte_views = false;

    for name in &reachable_items {
        if native_bodies.contains_key(name) {
            continue;
        }
        let Some(entry) = funcs.get(name) else {
            continue;
        };
        let Ok(func) = entry else {
            if debug {
                eprintln!("[bv] declined func: {}", name.as_str());
                raw_byte_derefs = true;
                scalar_byte_views = true;
                continue;
            }
            return ByteViewPlan {
                raw_byte_derefs: true,
                elide_scale_scalar: false,
            };
        };
        let Some(item) = table.get(name) else {
            if debug {
                eprintln!("[bv] missing item: {}", name.as_str());
                raw_byte_derefs = true;
                scalar_byte_views = true;
                continue;
            }
            return ByteViewPlan {
                raw_byte_derefs: true,
                elide_scale_scalar: false,
            };
        };
        let Some(interns) = item.jir.interns() else {
            if debug {
                eprintln!("[bv] missing interns: {}", name.as_str());
                raw_byte_derefs = true;
                scalar_byte_views = true;
                continue;
            }
            return ByteViewPlan {
                raw_byte_derefs: true,
                elide_scale_scalar: false,
            };
        };
        let (fn_raw, fn_scalar) = analyze_func(item, func, interns, table, debug);
        raw_byte_derefs |= fn_raw;
        scalar_byte_views |= fn_scalar;
        if raw_byte_derefs && !debug {
            return ByteViewPlan {
                raw_byte_derefs: true,
                elide_scale_scalar: false,
            };
        }
    }

    ByteViewPlan {
        raw_byte_derefs,
        elide_scale_scalar: !raw_byte_derefs && !scalar_byte_views,
    }
}

fn collect_const_refs(c: &ConstTree, interns: &Interns, out: &mut Vec<JsName>) {
    match c {
        ConstTree::Global { target, .. } => match target {
            GlobalReference::Function { link_name, .. }
            | GlobalReference::Static { link_name, .. } => {
                if let Some(s) = interns.str(*link_name) {
                    out.push(JsName::new(s.to_owned()));
                }
            }
            GlobalReference::Type(_) => {}
        },
        ConstTree::Dyn { data, vtable, .. } => {
            collect_const_refs(data, interns, out);
            if let Some(s) = interns.str(*vtable) {
                out.push(JsName::new(s.to_owned()));
            }
        }
        ConstTree::Reference(inner) => collect_const_refs(inner, interns, out),
        ConstTree::Aggregate(fields) | ConstTree::Slice(fields) => {
            for f in fields {
                collect_const_refs(f, interns, out);
            }
        }
        ConstTree::Variant { fields, .. } => {
            for f in fields {
                collect_const_refs(f, interns, out);
            }
        }
        ConstTree::Unit
        | ConstTree::Scalar { .. }
        | ConstTree::Str(_)
        | ConstTree::Address(_)
        | ConstTree::SliceAddress { .. } => {}
    }
}

fn collect_reachable_items(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    extra_roots: &[String],
) -> BTreeSet<JsName> {
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for (name, item) in table {
        if (item.linkage.root || extra_roots.iter().any(|r| r == name.as_str()))
            && visited.insert(name.clone())
        {
            queue.push_back(name.clone());
        }
    }

    while let Some(name) = queue.pop_front() {
        if let Some(body) = native_bodies.get(&name) {
            let mut idents = BTreeSet::new();
            for stmt in &body.statements {
                jsast::visit_idents(stmt, &mut |id| {
                    idents.insert(JsName::new(id.to_owned()));
                });
            }
            for r in idents {
                if table.contains_key(&r) && visited.insert(r.clone()) {
                    queue.push_back(r);
                }
            }
            continue;
        }
        let Some(item) = table.get(&name) else {
            continue;
        };
        if let (Some(Ok(func)), Some(interns)) = (funcs.get(&name), item.jir.interns()) {
            let reached = reachable_blocks(func);
            let mut refs = Vec::new();
            let mut has_drop_or_host = false;
            for block in &func.blocks {
                if !reached.contains(&block.id) {
                    continue;
                }
                for op in &block.ops {
                    match &op.kind {
                        OpKind::Call {
                            callee: Callee::Direct { link_name, .. },
                            ..
                        } => {
                            if let Some(s) = interns.str(*link_name) {
                                refs.push(JsName::new(s.to_owned()));
                            }
                        }
                        OpKind::Const(c) => collect_const_refs(c, interns, &mut refs),
                        OpKind::Drop { .. } | OpKind::Host { .. } => {
                            has_drop_or_host = true;
                        }
                        _ => {}
                    }
                }
            }
            if has_drop_or_host {
                // Conservatively retain drop glue, closures, and vtables referenced by this item,
                // without pulling unrelated dead calls like `swap_nonoverlapping`.
                for r in &item.refs {
                    let s = r.as_str();
                    if s.contains("drop_in_place")
                        || s.starts_with("vtable$")
                        || s.contains("closure")
                        || !funcs.contains_key(r)
                    {
                        refs.push(r.clone());
                    }
                }
            }
            for r in refs {
                if table.contains_key(&r) && visited.insert(r.clone()) {
                    queue.push_back(r);
                }
            }
        } else {
            for r in &item.refs {
                if table.contains_key(r) && visited.insert(r.clone()) {
                    queue.push_back(r.clone());
                }
            }
        }
    }

    visited
}

/// Computes the set of reachable blocks in `func`, folding constant `Switch` branches such as
/// `if T::IS_ZST` and `if size_of::<T>() > 32` so dead stdlib branches do not pollute analysis.
fn reachable_blocks(func: &Func) -> BTreeSet<RBlockId> {
    let mut consts: BTreeMap<ValueId, u128> = BTreeMap::new();
    for block in &func.blocks {
        for op in &block.ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Const(ConstTree::Scalar { bits, .. }) => {
                    consts.insert(res, *bits);
                }
                OpKind::Copy(v) | OpKind::Move(v) => {
                    if let Some(&c) = consts.get(v) {
                        consts.insert(res, c);
                    }
                }
                OpKind::Unary {
                    op: UnaryOp::Not,
                    v,
                    ..
                } => {
                    if let Some(&c) = consts.get(v) {
                        if c <= 1 {
                            consts.insert(res, 1 - c);
                        }
                    }
                }
                OpKind::Binary { op: bop, l, r, .. } => {
                    if let (Some(&lv), Some(&rv)) = (consts.get(l), consts.get(r)) {
                        let folded = match bop {
                            BinaryOp::Eq => Some(u128::from(lv == rv)),
                            BinaryOp::Ne => Some(u128::from(lv != rv)),
                            BinaryOp::Lt => Some(u128::from(lv < rv)),
                            BinaryOp::Le => Some(u128::from(lv <= rv)),
                            BinaryOp::Gt => Some(u128::from(lv > rv)),
                            BinaryOp::Ge => Some(u128::from(lv >= rv)),
                            _ => None,
                        };
                        if let Some(c) = folded {
                            consts.insert(res, c);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut reached = BTreeSet::new();
    if func.blocks.is_empty() {
        return reached;
    }
    let entry = func.blocks[0].id;
    let block_map: BTreeMap<RBlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    let mut queue = VecDeque::new();
    reached.insert(entry);
    queue.push_back(entry);

    while let Some(bid) = queue.pop_front() {
        let Some(&idx) = block_map.get(&bid) else {
            continue;
        };
        let block = &func.blocks[idx];
        match &block.term {
            RTerm::Goto(target, _) => {
                if reached.insert(*target) {
                    queue.push_back(*target);
                }
            }
            RTerm::Switch {
                scrut,
                cases,
                default,
                ..
            } => {
                if let Some(&c) = consts.get(scrut) {
                    let target = cases
                        .iter()
                        .find(|(val, _, _)| *val == c)
                        .map_or(default.0, |(_, t, _)| *t);
                    if reached.insert(target) {
                        queue.push_back(target);
                    }
                } else {
                    for (_, target, _) in cases {
                        if reached.insert(*target) {
                            queue.push_back(*target);
                        }
                    }
                    if reached.insert(default.0) {
                        queue.push_back(default.0);
                    }
                }
            }
            RTerm::Invoke { normal, unwind } => {
                if reached.insert(normal.0) {
                    queue.push_back(normal.0);
                }
                if reached.insert(unwind.0) {
                    queue.push_back(unwind.0);
                }
            }
            RTerm::Return(_) | RTerm::Throw(_) | RTerm::Unreachable => {}
        }
    }
    reached
}

/// Returns `(raw_byte_derefs, scalar_byte_views)` for a single function.
fn analyze_func(
    item: &LinkItem,
    func: &Func,
    interns: &Interns,
    items: &BTreeMap<JsName, LinkItem>,
    debug: bool,
) -> (bool, bool) {
    let ty_table = &interns.types;
    let in_stdlib_alloc = is_stdlib_alloc_wrapper(item);
    let reached = reachable_blocks(func);
    let (uses, cell_reads) = collect_value_uses(func, &reached);
    let zero_consts = collect_zero_consts(func, &reached);

    let mut raw_bv = false;
    let mut scalar_bv = false;
    let mut has_unscale_uninit = false;
    let mut has_byte_mem_op = false;
    let mut wide_rawvec_byte_ptrs: Vec<(ValueId, u64)> = Vec::new();
    let mut non_scalar_scales: Vec<ValueId> = Vec::new();
    let mut scalar_scales: Vec<(ValueId, bool)> = Vec::new(); // (val, is_float)

    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            match &op.kind {
                OpKind::Cast { kind, from, to, .. } => {
                    let hit = classify_cast(
                        ty_table,
                        *kind,
                        *from,
                        *to,
                        op.result,
                        &mut non_scalar_scales,
                        &mut scalar_scales,
                        &mut has_unscale_uninit,
                    );
                    if let Some((is_raw, reason)) = hit {
                        if debug {
                            eprintln!(
                                "[bv] {} ({}): {reason}",
                                item.name.as_str(),
                                item.debug_path
                            );
                        }
                        if is_raw {
                            raw_bv = true;
                        } else {
                            scalar_bv = true;
                        }
                    }
                }
                OpKind::SliceView { ptr, .. } => {
                    if let Some(res) = op.result {
                        let res_ty = func.values[res.0 as usize].ty;
                        if is_byte_slice_or_str_ptr(ty_table, res_ty) {
                            let ptr_ty = func.values[ptr.0 as usize].ty;
                            if ptr_pointee_is_wide_or_erased(ty_table, ptr_ty) {
                                if debug {
                                    eprintln!(
                                        "[bv] {} ({}): slice_view",
                                        item.name.as_str(),
                                        item.debug_path
                                    );
                                }
                                raw_bv = true;
                            }
                        }
                    }
                }
                OpKind::Intrinsic { name, args, .. } => {
                    let intr = interns.str(*name).unwrap_or("");
                    match intr {
                        "aggregate_raw_ptr" => {
                            if let (Some(res), Some(data)) = (op.result, args.first()) {
                                let res_ty = func.values[res.0 as usize].ty;
                                if is_byte_ptr_or_slice_or_str(ty_table, res_ty) {
                                    let data_ty = func.values[data.0 as usize].ty;
                                    if ptr_pointee_is_wide_or_erased(ty_table, data_ty) {
                                        if debug {
                                            eprintln!(
                                                "[bv] {} ({}): aggregate_raw_ptr",
                                                item.name.as_str(),
                                                item.debug_path
                                            );
                                        }
                                        raw_bv = true;
                                    }
                                }
                            }
                        }
                        "copy"
                        | "copy_nonoverlapping"
                        | "volatile_copy_memory"
                        | "volatile_copy_nonoverlapping_memory"
                        | "write_bytes"
                        | "volatile_set_memory"
                        | "compare_bytes" => {
                            if let Some(first) = args.first() {
                                let ptr_ty = func.values[first.0 as usize].ty;
                                if is_byte_ptr(ty_table, ptr_ty) {
                                    has_byte_mem_op = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                OpKind::Call { callee, .. } => {
                    if !in_stdlib_alloc && is_direct_alloc_callee(callee, interns, items) {
                        if debug {
                            eprintln!(
                                "[bv] {} ({}): direct_alloc",
                                item.name.as_str(),
                                item.debug_path
                            );
                        }
                        raw_bv = true;
                    }
                }
                OpKind::Field { base, .. } => {
                    if !in_stdlib_alloc {
                        if let Some(res) = op.result {
                            let base_ty = func.values[base.0 as usize].ty;
                            let res_ty = func.values[res.0 as usize].ty;
                            if let Some(elem_size) =
                                wide_vec_or_rawvec_elem_size(ty_table, interns, base_ty)
                            {
                                if is_byte_ptr_or_rawvec_inner(ty_table, interns, res_ty) {
                                    wide_rawvec_byte_ptrs.push((res, elem_size));
                                }
                            }
                        }
                    }
                }
                OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. } => {
                    if path_derefs_byte_ptr(func, ty_table, root, path) {
                        has_byte_mem_op = true;
                    }
                    if !in_stdlib_alloc {
                        if let Some(res) = op.result {
                            let res_ty = func.values[res.0 as usize].ty;
                            if is_byte_ptr_or_rawvec_inner(ty_table, interns, res_ty) {
                                if let Some(elem_size) =
                                    root_wide_vec_elem_size(func, ty_table, interns, root, path)
                                {
                                    wide_rawvec_byte_ptrs.push((res, elem_size));
                                }
                            }
                        }
                    }
                }
                OpKind::Deref(ptr) | OpKind::StoreThrough { ptr, .. } => {
                    let ptr_ty = func.values[ptr.0 as usize].ty;
                    if is_byte_ptr(ty_table, ptr_ty) {
                        has_byte_mem_op = true;
                    }
                }
                _ => {}
            }
        }
    }

    if has_unscale_uninit && has_byte_mem_op {
        if debug {
            eprintln!(
                "[bv] {} ({}): unscale_uninit_with_byte_mem",
                item.name.as_str(),
                item.debug_path
            );
        }
        raw_bv = true;
    }

    for val in non_scalar_scales {
        if let Some(why) = scale_value_escapes(
            func,
            ty_table,
            interns,
            items,
            &uses,
            &cell_reads,
            &zero_consts,
            val,
            None,
        ) {
            if debug {
                eprintln!(
                    "[bv] {} ({}): non_scalar_scale_escape ({why})",
                    item.name.as_str(),
                    item.debug_path
                );
            }
            raw_bv = true;
        }
    }

    for (val, is_float) in scalar_scales {
        if let Some(why) = scale_value_escapes(
            func,
            ty_table,
            interns,
            items,
            &uses,
            &cell_reads,
            &zero_consts,
            val,
            Some(!is_float),
        ) {
            if debug {
                eprintln!(
                    "[bv] {} ({}): scalar_scale_escape ({why})",
                    item.name.as_str(),
                    item.debug_path
                );
            }
            scalar_bv = true;
        }
    }

    for (val, elem_size) in wide_rawvec_byte_ptrs {
        if rawvec_byte_ptr_escapes(
            func,
            ty_table,
            interns,
            items,
            &uses,
            &cell_reads,
            val,
            elem_size,
        ) {
            if debug {
                eprintln!(
                    "[bv] {} ({}): rawvec_byte_ptr_escape",
                    item.name.as_str(),
                    item.debug_path
                );
            }
            raw_bv = true;
        }
    }

    (raw_bv, scalar_bv)
}

fn is_stdlib_alloc_wrapper(item: &LinkItem) -> bool {
    let path = item.debug_path.as_str();
    if path.starts_with("__rustc::")
        || path.starts_with("core::alloc::")
        || path.contains("::__rust_alloc")
        || path.contains("::__rust_realloc")
    {
        return true;
    }
    if item.crate_name == "alloc" {
        return path.starts_with("alloc::alloc::")
            || path.starts_with("alloc::Global::")
            || path.starts_with("<alloc::Global as ")
            || path.starts_with("<alloc::alloc::Global as ")
            || path.starts_with("alloc::raw_vec::")
            || path.starts_with("raw_vec::RawVecInner")
            || path.starts_with("alloc::boxed::")
            || path.starts_with("alloc::rc::")
            || path.starts_with("alloc::sync::");
    }
    path.starts_with("alloc::alloc::")
        || path.starts_with("alloc::Global::")
        || path.starts_with("<alloc::Global as core::alloc::Allocator>::")
        || path.starts_with("<alloc::alloc::Global as core::alloc::Allocator>::")
        || path.starts_with("alloc::raw_vec::")
        || path.starts_with("raw_vec::RawVecInner")
        || path.starts_with("alloc::boxed::")
        || path.starts_with("alloc::rc::Rc::")
        || path.starts_with("alloc::sync::Arc::")
}

fn is_direct_alloc_callee(
    callee: &Callee,
    interns: &Interns,
    items: &BTreeMap<JsName, LinkItem>,
) -> bool {
    match callee {
        Callee::Runtime { member, .. } => matches!(
            interns.str(*member).unwrap_or(""),
            "alloc" | "alloc_zeroed" | "realloc"
        ),
        Callee::Foreign { symbol, .. } => {
            let sym = interns.str(*symbol).unwrap_or("");
            (sym.contains("__rust_alloc") && !sym.contains("__rust_alloc_error_handler"))
                || sym.contains("__rust_realloc")
        }
        Callee::Direct { link_name, .. } => {
            let name = interns.str(*link_name).unwrap_or("");
            if (name.contains("__rust_alloc") && !name.contains("__rust_alloc_error_handler"))
                || name.contains("__rust_realloc")
            {
                return true;
            }
            if let Some(target) = items.get(&JsName::new(name.to_owned())) {
                let dp = target.debug_path.as_str();
                return matches!(
                    dp,
                    "alloc::alloc::alloc" | "alloc::alloc::alloc_zeroed" | "alloc::alloc::realloc"
                ) || dp.starts_with("alloc::alloc::Global::alloc_impl")
                    || dp.starts_with("alloc::Global::alloc_impl")
                    || dp.starts_with("alloc::alloc::Global::grow_impl")
                    || dp.starts_with("alloc::Global::grow_impl")
                    || dp
                        .starts_with("<alloc::alloc::Global as core::alloc::Allocator>::allocate")
                    || dp.starts_with("<alloc::Global as core::alloc::Allocator>::allocate")
                    || dp.starts_with("<alloc::alloc::Global as core::alloc::Allocator>::grow")
                    || dp.starts_with("<alloc::Global as core::alloc::Allocator>::grow");
            }
            false
        }
        _ => false,
    }
}

fn is_byte_int(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        value::kind(table, table.peeled(ty)),
        Some(TypeKind::Integer { bits: 8, .. })
    )
}

fn scalar_view_kind(table: &TypeTable, ty: TypeId) -> Option<bool> {
    match value::kind(table, table.peeled(ty)) {
        Some(
            TypeKind::Integer {
                bits: 16 | 32 | 64 | 128,
                ..
            }
            | TypeKind::Char,
        ) => Some(false),
        Some(TypeKind::Float { bits: 32 | 64 }) => Some(true),
        _ => None,
    }
}

fn is_byte_ptr(table: &TypeTable, ty: TypeId) -> bool {
    value::pointee_of(table, ty).is_some_and(|p| is_byte_int(table, p))
}

fn is_byte_slice_or_str_ptr(table: &TypeTable, ty: TypeId) -> bool {
    let Some(p) = value::pointee_of(table, ty) else {
        return false;
    };
    match value::kind(table, table.peeled(p)) {
        Some(TypeKind::Slice(e)) => is_byte_int(table, *e),
        Some(TypeKind::Str) => true,
        _ => false,
    }
}

fn is_byte_ptr_or_slice_or_str(table: &TypeTable, ty: TypeId) -> bool {
    is_byte_ptr(table, ty) || is_byte_slice_or_str_ptr(table, ty)
}

fn ptr_pointee_is_wide_or_erased(table: &TypeTable, ty: TypeId) -> bool {
    let Some(p) = value::pointee_of(table, ty) else {
        return false;
    };
    let peeled = table.peeled(p);
    match value::kind(table, peeled) {
        Some(TypeKind::Slice(e) | TypeKind::Array { element: e, .. }) => {
            value::size_of(table, *e).is_none_or(|s| s != 1)
        }
        Some(TypeKind::Str) => false,
        _ => value::size_of(table, peeled).is_none_or(|s| s != 1),
    }
}

fn path_derefs_byte_ptr(func: &Func, table: &TypeTable, root: &Root, path: &[Proj]) -> bool {
    let mut ty = match root {
        Root::Value(v) => func.values[v.0 as usize].ty,
        Root::Cell(c) => func.cells[c.0 as usize].ty,
    };
    for proj in path {
        match proj {
            Proj::Deref => {
                if is_byte_ptr(table, ty) || is_byte_slice_or_str_ptr(table, ty) {
                    return true;
                }
                let Some(pointee) = value::pointee_of(table, ty) else {
                    return false;
                };
                ty = pointee;
            }
            Proj::Field { ty: field_ty, .. } => ty = *field_ty,
            Proj::Index(_) | Proj::ConstantIndex { .. } => {
                match value::kind(table, table.peeled(ty)) {
                    Some(TypeKind::Array { element, .. } | TypeKind::Slice(element)) => {
                        ty = *element;
                    }
                    _ => return false,
                }
            }
            Proj::Subslice { .. } | Proj::Downcast { .. } => {}
        }
    }
    false
}

/// Returns `Some((is_raw_bv, reason))` if the cast immediately requires byte-view machinery.
#[allow(clippy::too_many_arguments)]
fn classify_cast(
    table: &TypeTable,
    kind: CastKind,
    from: TypeId,
    to: TypeId,
    result: Option<ValueId>,
    non_scalar_scales: &mut Vec<ValueId>,
    scalar_scales: &mut Vec<(ValueId, bool)>,
    has_unscale_uninit: &mut bool,
) -> Option<(bool, &'static str)> {
    let (Some(fp), Some(tp)) = (value::pointee_of(table, from), value::pointee_of(table, to))
    else {
        if kind == CastKind::Transmute
            && (is_byte_ptr_or_slice_or_str(table, from) || is_byte_ptr_or_slice_or_str(table, to))
            && (ptr_pointee_is_wide_or_erased(table, from)
                || ptr_pointee_is_wide_or_erased(table, to))
        {
            return Some((true, "transmute_wide_byte"));
        }
        return None;
    };

    let from_p = table.peeled(fp);
    let to_p = table.peeled(tp);
    let from_size = value::size_of(table, from_p);
    let to_size = value::size_of(table, to_p);

    if table.crosses_transparent_union(fp) && from_size == Some(1) && to_size.is_some_and(|s| s > 1)
    {
        *has_unscale_uninit = true;
    }

    // Erased `()` -> 1-byte pointer (`thin(_, 1)`).
    if from_size == Some(0) && is_byte_int(table, to_p) {
        return Some((false, "thin_erased_to_byte"));
    }

    // `u8` <-> `i8` (`byte_sign`).
    if let (
        Some(TypeKind::Integer {
            signed: s1,
            bits: 8,
        }),
        Some(TypeKind::Integer {
            signed: s2,
            bits: 8,
        }),
    ) = (value::kind(table, from_p), value::kind(table, to_p))
    {
        if s1 != s2 {
            return Some((false, "byte_sign"));
        }
    }

    // Slice/Array of wide elements -> 1-byte pointer.
    let eff_from = match value::kind(table, from_p) {
        Some(TypeKind::Slice(e) | TypeKind::Array { element: e, .. }) => table.peeled(*e),
        _ => from_p,
    };
    let eff_from_size = value::size_of(table, eff_from);

    if to_size == Some(1) && !table.is_indirect(to_p) && eff_from_size.is_some_and(|s| s > 1) {
        if is_byte_int(table, to_p) {
            if let Some(is_float) = scalar_view_kind(table, eff_from) {
                if let Some(res) = result {
                    scalar_scales.push((res, is_float));
                    return None;
                }
                return Some((false, "scale_scalar_no_result"));
            }
        }
        if let Some(res) = result {
            non_scalar_scales.push(res);
        } else {
            return Some((true, "scale_no_result"));
        }
    }

    None
}

fn wide_vec_or_rawvec_elem_size(table: &TypeTable, interns: &Interns, ty: TypeId) -> Option<u64> {
    let mut cur = table.peeled(ty);
    if let Some(p) = value::pointee_of(table, cur) {
        cur = table.peeled(p);
    }
    let desc = table.get(cur)?;
    let TypeKind::Adt { args, .. } = &desc.kind else {
        return None;
    };
    let is_vec_or_rawvec = matches!(
        desc.meta.candidate,
        rcgjs_object::jir::types::RepresentationCandidate::Vec
    ) || matches!(adt_short_name(table, interns, cur), "Vec" | "RawVec");
    if !is_vec_or_rawvec {
        return None;
    }
    let elem = *args.first()?;
    let size = value::size_of(table, elem)?;
    (size > 1).then_some(size)
}

fn adt_short_name<'a>(table: &'a TypeTable, interns: &'a Interns, ty: TypeId) -> &'a str {
    let Some(desc) = table.get(ty) else {
        return "";
    };
    if let TypeKind::Adt { variants, .. } = &desc.kind {
        if variants.len() == 1 {
            return interns.str(variants[0].name).unwrap_or("");
        }
    }
    ""
}

fn is_byte_ptr_or_rawvec_inner(table: &TypeTable, interns: &Interns, ty: TypeId) -> bool {
    let cur = table.peeled(ty);
    if is_byte_ptr(table, cur) {
        return true;
    }
    matches!(
        adt_short_name(table, interns, ty),
        "RawVecInner" | "RawVec" | "Unique" | "NonNull"
    ) || matches!(
        adt_short_name(table, interns, cur),
        "RawVecInner" | "RawVec"
    )
}

fn root_wide_vec_elem_size(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    root: &Root,
    path: &[Proj],
) -> Option<u64> {
    let mut ty = match root {
        Root::Value(v) => func.values[v.0 as usize].ty,
        Root::Cell(c) => func.cells[c.0 as usize].ty,
    };
    if let Some(sz) = wide_vec_or_rawvec_elem_size(table, interns, ty) {
        return Some(sz);
    }
    for proj in path {
        match proj {
            Proj::Deref => {
                ty = value::pointee_of(table, ty)?;
            }
            Proj::Field { ty: field_ty, .. } => {
                ty = *field_ty;
            }
            _ => return None,
        }
        if let Some(sz) = wide_vec_or_rawvec_elem_size(table, interns, ty) {
            return Some(sz);
        }
    }
    None
}

enum UseSite<'a> {
    Op(&'a Op),
    EdgeArg(ValueId),
    Return,
    OtherTerm,
}

fn collect_zero_consts(func: &Func, reached: &BTreeSet<RBlockId>) -> BTreeSet<ValueId> {
    let mut zeros = BTreeSet::new();
    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Const(ConstTree::Scalar { bits: 0, .. }) => {
                    zeros.insert(res);
                }
                OpKind::Copy(v) | OpKind::Move(v) if zeros.contains(v) => {
                    zeros.insert(res);
                }
                _ => {}
            }
        }
    }
    zeros
}

fn collect_value_uses<'a>(
    func: &'a Func,
    reached: &BTreeSet<RBlockId>,
) -> (
    BTreeMap<ValueId, Vec<UseSite<'a>>>,
    BTreeMap<CellId, Vec<ValueId>>,
) {
    let block_params: BTreeMap<RBlockId, &[ValueId]> = func
        .blocks
        .iter()
        .map(|b| (b.id, b.params.as_slice()))
        .collect();
    let mut map: BTreeMap<ValueId, Vec<UseSite<'a>>> = BTreeMap::new();
    let mut cell_reads: BTreeMap<CellId, Vec<ValueId>> = BTreeMap::new();

    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            for used in op.kind.uses() {
                map.entry(used).or_default().push(UseSite::Op(op));
            }
            if let Some(res) = op.result {
                match &op.kind {
                    OpKind::CellLoad(cell) => {
                        cell_reads.entry(*cell).or_default().push(res);
                    }
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Cell(cell),
                        ..
                    } => {
                        cell_reads.entry(*cell).or_default().push(res);
                    }
                    _ => {}
                }
            }
        }
        match &block.term {
            RTerm::Return(Some(v)) => {
                map.entry(*v).or_default().push(UseSite::Return);
            }
            RTerm::Throw(Some(v)) => {
                map.entry(*v).or_default().push(UseSite::OtherTerm);
            }
            RTerm::Return(None) | RTerm::Throw(None) | RTerm::Unreachable => {}
            RTerm::Switch { scrut, .. } => {
                map.entry(*scrut).or_default().push(UseSite::OtherTerm);
                for (target, args) in block.term.edges() {
                    if !reached.contains(&target) {
                        continue;
                    }
                    let params = block_params.get(&target).copied().unwrap_or(&[]);
                    for (idx, arg) in args.iter().enumerate() {
                        if let Some(&param) = params.get(idx) {
                            map.entry(*arg).or_default().push(UseSite::EdgeArg(param));
                        } else {
                            map.entry(*arg).or_default().push(UseSite::OtherTerm);
                        }
                    }
                }
            }
            RTerm::Goto(_, _) | RTerm::Invoke { .. } => {
                for (target, args) in block.term.edges() {
                    if !reached.contains(&target) {
                        continue;
                    }
                    let params = block_params.get(&target).copied().unwrap_or(&[]);
                    for (idx, arg) in args.iter().enumerate() {
                        if let Some(&param) = params.get(idx) {
                            map.entry(*arg).or_default().push(UseSite::EdgeArg(param));
                        } else {
                            map.entry(*arg).or_default().push(UseSite::OtherTerm);
                        }
                    }
                }
            }
        }
    }
    (map, cell_reads)
}

fn is_dealloc_or_rawvec_call(
    callee: &Callee,
    interns: &Interns,
    items: &BTreeMap<JsName, LinkItem>,
) -> bool {
    match callee {
        Callee::Runtime { member, .. } => matches!(
            interns.str(*member).unwrap_or(""),
            "dealloc" | "realloc" | "ptr_is_null"
        ),
        Callee::Foreign { symbol, .. } => {
            let sym = interns.str(*symbol).unwrap_or("");
            sym.contains("__rust_dealloc") || sym.contains("__rust_realloc")
        }
        Callee::Direct { link_name, .. } => {
            let name = interns.str(*link_name).unwrap_or("");
            if name.contains("__rust_dealloc") || name.contains("__rust_realloc") {
                return true;
            }
            if let Some(target) = items.get(&JsName::new(name.to_owned())) {
                let dp = target.debug_path.as_str();
                return dp.starts_with("alloc::raw_vec::")
                    || dp.starts_with("raw_vec::RawVecInner")
                    || dp.starts_with("alloc::alloc::Global::deallocate")
                    || dp.starts_with("alloc::Global::deallocate")
                    || dp.starts_with(
                        "<alloc::alloc::Global as core::alloc::Allocator>::deallocate",
                    )
                    || dp.starts_with("<alloc::Global as core::alloc::Allocator>::deallocate")
                    || dp.starts_with("alloc::alloc::dealloc")
                    || dp.starts_with("alloc::alloc::realloc")
                    || dp.starts_with("core::ptr::drop_in_place")
                    || dp.starts_with("core::ptr::non_null::NonNull")
                    || dp.starts_with("core::ptr::unique::Unique")
                    || dp.starts_with("core::ptr::const_ptr::<impl *const T>::is_null")
                    || dp.starts_with("core::ptr::mut_ptr::<impl *mut T>::is_null")
                    || dp.contains("RawVecInner")
                    || dp.contains("RawVec<");
            }
            false
        }
        Callee::Nop => true,
        _ => false,
    }
}

fn is_wrapper_aggregate(table: &TypeTable, interns: &Interns, ty: TypeId) -> bool {
    for candidate in [ty, table.peeled(ty)] {
        if matches!(
            adt_short_name(table, interns, candidate),
            "RawVecInner"
                | "RawVec"
                | "Vec"
                | "IntoIter"
                | "Box"
                | "Unique"
                | "NonNull"
                | "ManuallyDrop"
                | "MaybeUninit"
        ) {
            return true;
        }
        if let Some(desc) = table.get(candidate) {
            if matches!(
                desc.meta.candidate,
                rcgjs_object::jir::types::RepresentationCandidate::Vec
                    | rcgjs_object::jir::types::RepresentationCandidate::Box
            ) {
                return true;
            }
            if let TypeKind::Adt { variants, .. } = &desc.kind {
                if variants.len() == 2
                    && variants.iter().any(|v| interns.str(v.name) == Some("Some"))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// True if `ty` is an opaque stdlib container that may safely cross a function return boundary
/// (its internal byte pointer is guarded by Rule 5 in any caller that unpacks it).
fn is_returnable_container(table: &TypeTable, interns: &Interns, ty: TypeId) -> bool {
    for candidate in [ty, table.peeled(ty)] {
        if matches!(
            adt_short_name(table, interns, candidate),
            "RawVecInner" | "RawVec" | "Vec" | "IntoIter" | "Box"
        ) {
            return true;
        }
        if let Some(desc) = table.get(candidate) {
            if matches!(
                desc.meta.candidate,
                rcgjs_object::jir::types::RepresentationCandidate::Vec
                    | rcgjs_object::jir::types::RepresentationCandidate::Box
            ) {
                return true;
            }
        }
    }
    false
}

/// True if every use of `val` (the `i32` result of `compare_bytes`) is `val == 0` or `val != 0`.
fn only_used_for_zero_equality(
    uses: &BTreeMap<ValueId, Vec<UseSite<'_>>>,
    zero_consts: &BTreeSet<ValueId>,
    val: ValueId,
) -> bool {
    let mut visited = BTreeSet::new();
    let mut queue = vec![val];
    while let Some(v) = queue.pop() {
        if !visited.insert(v) {
            continue;
        }
        let Some(sites) = uses.get(&v) else {
            continue;
        };
        for site in sites {
            match site {
                UseSite::EdgeArg(param) => queue.push(*param),
                UseSite::Return | UseSite::OtherTerm => return false,
                UseSite::Op(op) => match &op.kind {
                    OpKind::Copy(_) | OpKind::Move(_) => {
                        if let Some(res) = op.result {
                            queue.push(res);
                        }
                    }
                    OpKind::Binary {
                        op: BinaryOp::Eq | BinaryOp::Ne,
                        l,
                        r,
                        ..
                    } if (*l == v && zero_consts.contains(r))
                        || (*r == v && zero_consts.contains(l)) => {}
                    _ => return false,
                },
            }
        }
    }
    true
}

/// Checks whether a `scale` / `scale_scalar` result escapes outside `RawVec`/`Vec`/`Box`/`IntoIter`
/// construction, `dealloc`/`realloc`, or (when `allow_int_eq_compare == Some(true)`)
/// `compare_bytes(a, b, n) == 0 / != 0`. Returns `Some(reason)` if it escapes.
#[allow(clippy::too_many_arguments)]
fn scale_value_escapes(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    items: &BTreeMap<JsName, LinkItem>,
    uses: &BTreeMap<ValueId, Vec<UseSite<'_>>>,
    cell_reads: &BTreeMap<CellId, Vec<ValueId>>,
    zero_consts: &BTreeSet<ValueId>,
    start: ValueId,
    allow_int_eq_compare: Option<bool>,
) -> Option<&'static str> {
    let mut visited = BTreeSet::new();
    let mut queue = vec![start];
    while let Some(v) = queue.pop() {
        if !visited.insert(v) {
            continue;
        }
        let Some(sites) = uses.get(&v) else {
            continue;
        };
        for site in sites {
            match site {
                UseSite::EdgeArg(param) => queue.push(*param),
                UseSite::Return => {
                    let ty = func.values[v.0 as usize].ty;
                    if !is_returnable_container(table, interns, ty) {
                        return Some("return");
                    }
                }
                UseSite::OtherTerm => return Some("term"),
                UseSite::Op(op) => match &op.kind {
                    OpKind::Move(_) | OpKind::Copy(_) => {
                        if let Some(res) = op.result {
                            queue.push(res);
                        }
                    }
                    OpKind::CellStore { cell, .. } => {
                        if let Some(reads) = cell_reads.get(cell) {
                            queue.extend(reads.iter().copied());
                        }
                    }
                    // Mutability cast `*mut u8 -> *const u8` (same signedness byte ptr).
                    OpKind::Cast { from, to, .. } if same_byte_ptr_sign(table, *from, *to) => {
                        if let Some(res) = op.result {
                            queue.push(res);
                        }
                    }
                    OpKind::Aggregate { ty, .. } if is_wrapper_aggregate(table, interns, *ty) => {
                        if let Some(res) = op.result {
                            queue.push(res);
                        }
                    }
                    OpKind::Field { .. } => {
                        if let Some(res) = op.result {
                            let res_ty = func.values[res.0 as usize].ty;
                            if is_wrapper_aggregate(table, interns, res_ty)
                                || is_byte_ptr_or_rawvec_inner(table, interns, res_ty)
                            {
                                queue.push(res);
                            }
                        }
                    }
                    OpKind::Call { callee, .. }
                        if is_dealloc_or_rawvec_call(callee, interns, items) => {}
                    OpKind::Intrinsic { name, .. }
                        if allow_int_eq_compare == Some(true)
                            && interns.str(*name) == Some("compare_bytes") =>
                    {
                        let Some(res) = op.result else {
                            return Some("compare_bytes_no_res");
                        };
                        if !only_used_for_zero_equality(uses, zero_consts, res) {
                            return Some("compare_bytes_ord");
                        }
                    }
                    OpKind::Drop { .. } => {}
                    OpKind::Call { .. } => return Some("call"),
                    OpKind::Aggregate { .. } => return Some("aggregate"),
                    OpKind::Cast { .. } => return Some("cast"),
                    OpKind::Intrinsic { .. } => return Some("intrinsic"),
                    OpKind::SetField { .. } => return Some("set_field"),
                    OpKind::StoreThrough { .. } => return Some("store_through"),
                    OpKind::BorrowView { .. } | OpKind::RawView { .. } => return Some("view"),
                    _ => return Some("other_op"),
                },
            }
        }
    }
    None
}

fn same_byte_ptr_sign(table: &TypeTable, from: TypeId, to: TypeId) -> bool {
    let (Some(fp), Some(tp)) = (value::pointee_of(table, from), value::pointee_of(table, to))
    else {
        return false;
    };
    matches!(
        (
            value::kind(table, table.peeled(fp)),
            value::kind(table, table.peeled(tp))
        ),
        (
            Some(TypeKind::Integer {
                bits: 8,
                signed: s1
            }),
            Some(TypeKind::Integer {
                bits: 8,
                signed: s2
            })
        ) if s1 == s2
    )
}

/// Checks whether a byte pointer projected out of a wide `Vec<T>` / `RawVec<T>` escapes
/// into any operation other than re-casting back to `T` (`unscale`/`unscale_scalar`),
/// rebuilding `RawVec`/`NonNull`, or calling `RawVecInner`/`dealloc`/`realloc`/`ptr_is_null`.
#[allow(clippy::too_many_arguments)]
fn rawvec_byte_ptr_escapes(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    items: &BTreeMap<JsName, LinkItem>,
    uses: &BTreeMap<ValueId, Vec<UseSite<'_>>>,
    cell_reads: &BTreeMap<CellId, Vec<ValueId>>,
    start: ValueId,
    elem_size: u64,
) -> bool {
    let mut visited = BTreeSet::new();
    let mut queue = vec![start];
    while let Some(v) = queue.pop() {
        if !visited.insert(v) {
            continue;
        }
        let Some(sites) = uses.get(&v) else {
            continue;
        };
        for site in sites {
            match site {
                UseSite::EdgeArg(param) => queue.push(*param),
                UseSite::Return => {
                    let ty = func.values[v.0 as usize].ty;
                    if !is_returnable_container(table, interns, ty) {
                        return true;
                    }
                }
                UseSite::OtherTerm => return true,
                UseSite::Op(op) => match &op.kind {
                    OpKind::Move(_) | OpKind::Copy(_) => {
                        if let Some(res) = op.result {
                            queue.push(res);
                        }
                    }
                    OpKind::CellStore { cell, .. } => {
                        if let Some(reads) = cell_reads.get(cell) {
                            queue.extend(reads.iter().copied());
                        }
                    }
                    OpKind::Field { .. } => {
                        if let Some(res) = op.result {
                            let res_ty = func.values[res.0 as usize].ty;
                            if is_byte_ptr_or_rawvec_inner(table, interns, res_ty) {
                                queue.push(res);
                            }
                        }
                    }
                    OpKind::BorrowView { .. } | OpKind::RawView { .. } => {
                        if let Some(res) = op.result {
                            let res_ty = func.values[res.0 as usize].ty;
                            if is_byte_ptr_or_rawvec_inner(table, interns, res_ty) {
                                queue.push(res);
                            }
                        }
                    }
                    OpKind::Aggregate { ty, .. } if is_wrapper_aggregate(table, interns, *ty) => {
                        if let Some(res) = op.result {
                            queue.push(res);
                        }
                    }
                    OpKind::Cast { from, to, .. } => {
                        if same_byte_ptr_sign(table, *from, *to) {
                            if let Some(res) = op.result {
                                queue.push(res);
                            }
                            continue;
                        }
                        let ok = value::pointee_of(table, *to)
                            .and_then(|p| value::size_of(table, p))
                            .is_some_and(|s| s == elem_size);
                        if !ok {
                            return true;
                        }
                    }
                    OpKind::Call { callee, .. }
                        if is_dealloc_or_rawvec_call(callee, interns, items) => {}
                    OpKind::Drop { .. } => {}
                    _ => return true,
                },
            }
        }
    }
    false
}
