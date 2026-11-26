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
//!    or same-type whole-element integer/char equality checks (`compare_bytes(a, b, n) == 0` /
//!    `!= 0`), the JIR lowerer can emit Proxy-free `__rt.scale(p, size)` instead of
//!    `__rt.scale_scalar(p, size, kind)`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::OnceLock;

use rcgjs_object::jir::decline::Decline;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{CellId, Func, Op, OpKind, Proj, RBlockId, RTerm, Root, ValueId};
use rcgjs_object::jir::semantic::{
    BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DropGlue, GlobalReference, Terminator,
    UnaryOp,
};
use rcgjs_object::jir::types::{RepresentationCandidate, TypeId, TypeKind, TypeTable};

use crate::analysis::reach::{self, ReachPolicy};
use crate::item::{JsName, LinkItem};
use crate::lower::js::value;
use crate::plan::native_types::NativeBody;

/// Whole-program lowering policy for lazy byte views.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ByteViewPlan {
    /// Emit `__rt.read_byte*`/`__rt.write_byte*` on 1-byte dereferences and retain
    /// `__rt._bv_enable` in the emitted program.
    pub raw_byte_derefs: bool,
    /// Replace non-escaping `scale_scalar` casts with Proxy-free `__rt.scale`.
    pub elide_scale_scalar: bool,
}

/// Scalar element identity for a non-float `scale_scalar` cast.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScalarOrigin {
    bits: u32,
    signed: bool,
    is_char: bool,
    size: u64,
}

/// Classification of a wide scalar type that lowers to `scale_scalar`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScalarView {
    IntOrChar(ScalarOrigin),
    Float,
}

impl ScalarView {
    fn int_or_char(self) -> Option<ScalarOrigin> {
        match self {
            Self::IntOrChar(origin) => Some(origin),
            Self::Float => None,
        }
    }
}

fn bv_debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("RCGJS_BV_DEBUG").is_some())
}

/// Computes the conservative whole-program [`ByteViewPlan`] over items reachable from program roots.
pub fn analyze_reachable(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    extra_roots: &[String],
) -> ByteViewPlan {
    let std_crate_ids = collect_std_crate_ids(table);
    let reachable_items = collect_reachable_items(table, funcs, native_bodies, extra_roots);
    let debug = bv_debug_enabled();
    let mut raw_byte_derefs = false;
    let mut scalar_byte_views = false;

    for name in &reachable_items {
        if native_bodies.contains_key(name) {
            continue;
        }
        if table
            .get(name)
            .is_some_and(|item| reach::is_host_delegated_body(item, true))
        {
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
        let (fn_raw, fn_scalar) = analyze_func(item, func, interns, table, &std_crate_ids, debug);
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

/// Collects the unforgeable `DefinitionId::crate_id` values belonging to `core` and `alloc`.
fn collect_std_crate_ids(table: &BTreeMap<JsName, LinkItem>) -> BTreeSet<u64> {
    let mut ids = BTreeSet::new();
    for item in table.values() {
        let Some(body) = item.jir.body.as_ref() else {
            continue;
        };
        if item.crate_name == "core" || item.crate_name == "alloc" {
            ids.insert(body.item.definition.crate_id);
        }
        for block in &body.blocks {
            if let Terminator::Call(call) = &block.terminator {
                if matches!(call.semantic, CallSemantic::Standard(_)) {
                    if let Callee::Direct {
                        item: callee_item, ..
                    } = &call.callee
                    {
                        ids.insert(callee_item.definition.crate_id);
                    }
                }
            }
        }
        let Some(interns) = item.jir.interns() else {
            continue;
        };
        // Harvest `alloc` and `core` crate_ids from rustc-tagged Vec/Box/String ADTs and their
        // known internal stdlib wrapper fields (`RawVec`, `RawVecInner`, `Unique`, `NonNull`).
        let mut work = Vec::new();
        for desc in interns.types.descriptors() {
            if matches!(
                desc.meta.candidate,
                RepresentationCandidate::Vec
                    | RepresentationCandidate::Box
                    | RepresentationCandidate::String
            ) {
                work.push(desc.id);
            }
        }
        let mut seen = BTreeSet::new();
        while let Some(ty_id) = work.pop() {
            let ty_id = interns.types.peeled(ty_id);
            if !seen.insert(ty_id) {
                continue;
            }
            let Some(desc) = interns.types.get(ty_id) else {
                continue;
            };
            let TypeKind::Adt { def, variants, .. } = &desc.kind else {
                continue;
            };
            ids.insert(def.crate_id);
            if variants.len() == 1 {
                for field in &variants[0].fields {
                    let fty = interns.types.peeled(field.ty);
                    if matches!(
                        adt_short_name(&interns.types, interns, fty),
                        "RawVec" | "RawVecInner" | "Unique" | "NonNull" | "PhantomData"
                    ) {
                        work.push(fty);
                    }
                }
            }
        }
    }
    ids
}

fn collect_const_refs<'a>(c: &ConstTree, interns: &'a Interns, out: &mut BTreeSet<&'a str>) {
    match c {
        ConstTree::Global { target, .. }
        | ConstTree::StaticField { target, .. }
        | ConstTree::StaticSlice { target, .. } => match target {
            GlobalReference::Function { link_name, .. }
            | GlobalReference::Static { link_name, .. } => {
                if let Some(s) = interns.str(*link_name) {
                    out.insert(s);
                }
            }
            GlobalReference::Type(_) => {}
        },
        ConstTree::Dyn { data, vtable, .. } => {
            collect_const_refs(data, interns, out);
            if let Some(s) = interns.str(*vtable) {
                out.insert(s);
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

struct ByteViewReachPolicy<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &'a BTreeMap<JsName, NativeBody>,
}

impl ReachPolicy<JsName> for ByteViewReachPolicy<'_> {
    fn expand(&mut self, name: &JsName, enqueue: &mut dyn FnMut(JsName)) {
        if let Some(body) = self.native_bodies.get(name) {
            for ident in reach::native_body_idents(body) {
                if let Some((key, _)) = self.table.get_key_value(ident.as_str()) {
                    enqueue(key.clone());
                }
            }
            return;
        }
        let Some(item) = self.table.get(name) else {
            return;
        };
        if reach::is_host_delegated_body(item, true) {
            return;
        }
        if let (Some(Ok(func)), Some(interns)) = (self.funcs.get(name), item.jir.interns()) {
            let reached = reachable_blocks(func, &interns.types);
            let mut reachable_refs: BTreeSet<&str> = BTreeSet::new();
            let mut has_closure_coercion = false;
            let mut has_host = false;

            for block in &func.blocks {
                if !reached.contains(&block.id) {
                    continue;
                }
                for op in &block.ops {
                    match &op.kind {
                        OpKind::Call { callee, host, .. } => {
                            if host.is_some() {
                                has_host = true;
                            }
                            match callee {
                                Callee::Direct { link_name, .. } => {
                                    if let Some(s) = interns.str(*link_name) {
                                        reachable_refs.insert(s);
                                    }
                                }
                                Callee::Foreign { symbol, .. } => {
                                    if let Some(s) = interns.str(*symbol) {
                                        reachable_refs.insert(s);
                                    }
                                }
                                Callee::Intrinsic {
                                    fallback: Some(d), ..
                                } => {
                                    if let Some(s) = interns.str(d.link_name) {
                                        if item.refs.contains(s) {
                                            reachable_refs.insert(s);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        OpKind::Intrinsic {
                            name,
                            fallback,
                            args,
                            ..
                        } => {
                            if interns.str(*name) == Some("const_eval_select") {
                                if let Some(rt_arg) = args.get(2) {
                                    let ty = func.values[rt_arg.0 as usize].ty;
                                    if let Some(TypeKind::FnDef {
                                        instance: Some(d), ..
                                    }) = value::kind(&interns.types, interns.types.peeled(ty))
                                    {
                                        if let Some(s) = interns.str(d.link_name) {
                                            reachable_refs.insert(s);
                                        }
                                    }
                                }
                            }
                            if let Some(d) = fallback {
                                if let Some(s) = interns.str(d.link_name) {
                                    if item.refs.contains(s) {
                                        reachable_refs.insert(s);
                                    }
                                }
                            }
                        }
                        OpKind::Drop {
                            glue: DropGlue::Direct { link_name, .. },
                            ..
                        } => {
                            if let Some(s) = interns.str(*link_name) {
                                reachable_refs.insert(s);
                            }
                        }
                        OpKind::Const(c) => {
                            collect_const_refs(c, interns, &mut reachable_refs);
                        }
                        OpKind::Cast { from, .. } => {
                            match value::kind(&interns.types, interns.types.peeled(*from)) {
                                Some(TypeKind::FnDef {
                                    instance: Some(d), ..
                                }) => {
                                    if let Some(s) = interns.str(d.link_name) {
                                        reachable_refs.insert(s);
                                    }
                                }
                                Some(TypeKind::Closure { .. }) => {
                                    has_closure_coercion = true;
                                }
                                _ => {}
                            }
                        }
                        OpKind::Host { .. } => {
                            has_host = true;
                        }
                        _ => {}
                    }
                }
            }

            // Always follow vtables, statics (non-func items), closure shims when a closure is
            // coerced in a reachable block, and all refs when a host op is reachable — without
            // resurrecting dead direct calls already eliminated by JIR-opt or constant folding.
            for r in &item.refs {
                let s = r.as_str();
                if has_host
                    || s.starts_with("vtable$")
                    || !self.funcs.contains_key(r)
                    || (has_closure_coercion && s.contains("closure"))
                {
                    reachable_refs.insert(s);
                }
            }

            for r in reachable_refs {
                if let Some((key, _)) = self.table.get_key_value(r) {
                    enqueue(key.clone());
                }
            }
        } else {
            for r in &item.refs {
                if self.table.contains_key(r) {
                    enqueue(r.clone());
                }
            }
        }
    }
}

fn collect_reachable_items(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    extra_roots: &[String],
) -> BTreeSet<JsName> {
    let seeds = reach::root_seeds(table, extra_roots, false);
    let mut policy = ByteViewReachPolicy {
        table,
        funcs,
        native_bodies,
    };
    reach::walk_reachable(seeds, &mut policy)
}

/// Computes the set of reachable blocks in `func`, folding constant `Switch` branches such as
/// `if T::IS_ZST` and `if size_of::<T>() > 32` so dead stdlib branches do not pollute analysis.
///
/// Only folds `UnaryOp::Not` on `bool` and comparison ops on `bool` / unsigned integers so
/// integer bitwise `!`, signed integer comparisons, and floating-point comparisons are never
/// mis-folded.
fn reachable_blocks(func: &Func, ty_table: &TypeTable) -> BTreeSet<RBlockId> {
    let mut consts: Vec<Option<u128>> = vec![None; func.values.len()];
    for block in &func.blocks {
        for op in &block.ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Const(ConstTree::Scalar { bits, .. }) => {
                    consts[res.0 as usize] = Some(*bits);
                }
                OpKind::Copy(v) | OpKind::Move(v) => {
                    if let Some(c) = consts[v.0 as usize] {
                        consts[res.0 as usize] = Some(c);
                    }
                }
                OpKind::Unary {
                    op: UnaryOp::Not,
                    v,
                    ty,
                } => {
                    if matches!(
                        value::kind(ty_table, ty_table.peeled(*ty)),
                        Some(TypeKind::Bool)
                    ) {
                        if let Some(c) = consts[v.0 as usize] {
                            if c <= 1 {
                                consts[res.0 as usize] = Some(1 - c);
                            }
                        }
                    }
                }
                OpKind::Binary { op: bop, l, r, ty } => {
                    if matches!(
                        value::kind(ty_table, ty_table.peeled(*ty)),
                        Some(TypeKind::Bool | TypeKind::Integer { signed: false, .. })
                    ) {
                        if let (Some(lv), Some(rv)) = (consts[l.0 as usize], consts[r.0 as usize]) {
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
                                consts[res.0 as usize] = Some(c);
                            }
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
                let scrut_ty = func.values[scrut.0 as usize].ty;
                let can_fold_switch = matches!(
                    value::kind(ty_table, ty_table.peeled(scrut_ty)),
                    Some(TypeKind::Bool | TypeKind::Integer { .. } | TypeKind::Char)
                );
                if can_fold_switch {
                    if let Some(c) = consts[scrut.0 as usize] {
                        let target = cases
                            .iter()
                            .find(|(val, _, _)| *val == c)
                            .map_or(default.0, |(_, t, _)| *t);
                        if reached.insert(target) {
                            queue.push_back(target);
                        }
                        continue;
                    }
                }
                for (_, target, _) in cases {
                    if reached.insert(*target) {
                        queue.push_back(*target);
                    }
                }
                if reached.insert(default.0) {
                    queue.push_back(default.0);
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

/// Collected candidate pointer-cast sites in one function that need escape analysis.
#[derive(Default)]
struct CastHits {
    unscale_uninit_casts: Vec<(ValueId, Option<ValueId>)>,
    non_scalar_scales: Vec<(ValueId, u64, TypeId)>,
    scalar_scales: Vec<(ValueId, u64, TypeId, ScalarView)>,
}

/// Per-function context for byte-view escape and alignment checks.
struct ByteViewFuncCx<'a> {
    item: &'a LinkItem,
    func: &'a Func,
    table: &'a TypeTable,
    interns: &'a Interns,
    items: &'a BTreeMap<JsName, LinkItem>,
    std_crate_ids: &'a BTreeSet<u64>,
    in_stdlib_alloc: bool,
    reached: BTreeSet<RBlockId>,
    def_map: Vec<Option<&'a Op>>,
    uses: Vec<Vec<UseSite<'a>>>,
    cell_reads: Vec<Vec<ValueId>>,
    zero_consts: Vec<bool>,
    u_consts: Vec<Option<u128>>,
    scalar_origins: Vec<Option<ScalarOrigin>>,
    debug: bool,
}

/// Returns `(raw_byte_derefs, scalar_byte_views)` for a single function.
fn analyze_func(
    item: &LinkItem,
    func: &Func,
    interns: &Interns,
    items: &BTreeMap<JsName, LinkItem>,
    std_crate_ids: &BTreeSet<u64>,
    debug: bool,
) -> (bool, bool) {
    let ty_table = &interns.types;
    let in_stdlib_alloc = is_stdlib_alloc_wrapper(item, std_crate_ids);
    let reached = reachable_blocks(func, ty_table);
    let (uses, cell_reads) = collect_value_uses(func, &reached);
    let def_map = collect_def_map(func, &reached);
    let zero_consts = collect_zero_consts(func, &reached);
    let u_consts = collect_unsigned_consts(func, ty_table, &reached);

    let mut raw_bv = false;
    let mut scalar_bv = false;
    let mut cast_hits = CastHits::default();
    let mut wide_rawvec_byte_ptrs: Vec<(ValueId, u64)> = Vec::new();

    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            match &op.kind {
                OpKind::Cast { .. } => {
                    if let Some((is_raw, reason)) = classify_cast(ty_table, op, &mut cast_hits) {
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
                    if intr == "aggregate_raw_ptr" {
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
                            if let Some(elem_size) = wide_vec_or_rawvec_elem_size(
                                ty_table,
                                interns,
                                base_ty,
                                std_crate_ids,
                            ) {
                                if is_byte_ptr_or_rawvec_inner(
                                    ty_table,
                                    interns,
                                    res_ty,
                                    std_crate_ids,
                                ) {
                                    wide_rawvec_byte_ptrs.push((res, elem_size));
                                }
                            }
                        }
                    }
                }
                OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. }
                    if !in_stdlib_alloc =>
                {
                    if let Some(res) = op.result {
                        let res_ty = func.values[res.0 as usize].ty;
                        if is_byte_ptr_or_rawvec_inner(ty_table, interns, res_ty, std_crate_ids) {
                            if let Some(elem_size) = root_wide_vec_elem_size(
                                func,
                                ty_table,
                                interns,
                                root,
                                path,
                                std_crate_ids,
                            ) {
                                wide_rawvec_byte_ptrs.push((res, elem_size));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let scalar_scale_origins: Vec<(ValueId, ScalarView)> = cast_hits
        .scalar_scales
        .iter()
        .map(|&(val, _, _, view)| (val, view))
        .collect();
    let scalar_origins = propagate_scalar_origins(func, &reached, ty_table, &scalar_scale_origins);

    let cx = ByteViewFuncCx {
        item,
        func,
        table: ty_table,
        interns,
        items,
        std_crate_ids,
        in_stdlib_alloc,
        reached,
        def_map,
        uses,
        cell_reads,
        zero_consts,
        u_consts,
        scalar_origins,
        debug,
    };

    for (src, res) in cast_hits.unscale_uninit_casts {
        if !cx.unscale_uninit_source_is_private(src, res) {
            if cx.debug {
                eprintln!(
                    "[bv] {} ({}): unscale_uninit_shared_or_escaped",
                    cx.item.name.as_str(),
                    cx.item.debug_path
                );
            }
            raw_bv = true;
        }
    }

    for (val, elem_size, eff_from) in cast_hits.non_scalar_scales {
        if let Some(why) = cx.scale_value_escapes(val, elem_size, eff_from, None) {
            if cx.debug {
                eprintln!(
                    "[bv] {} ({}): non_scalar_scale_escape ({why})",
                    cx.item.name.as_str(),
                    cx.item.debug_path
                );
            }
            raw_bv = true;
        }
    }

    for (val, elem_size, eff_from, view) in cast_hits.scalar_scales {
        if let Some(why) = cx.scale_value_escapes(val, elem_size, eff_from, view.int_or_char()) {
            if cx.debug {
                eprintln!(
                    "[bv] {} ({}): scalar_scale_escape ({why})",
                    cx.item.name.as_str(),
                    cx.item.debug_path
                );
            }
            scalar_bv = true;
        }
    }

    for (val, elem_size) in wide_rawvec_byte_ptrs {
        if cx.rawvec_byte_ptr_escapes(val, elem_size) {
            if cx.debug {
                eprintln!(
                    "[bv] {} ({}): rawvec_byte_ptr_escape",
                    cx.item.name.as_str(),
                    cx.item.debug_path
                );
            }
            raw_bv = true;
        }
    }

    (raw_bv, scalar_bv)
}

impl<'a> ByteViewFuncCx<'a> {
    /// Checks whether an `unscale_uninit` cast's source pointer comes from a function-local cell
    /// (`OpKind::CellAlloc`) that is NEVER observed as bytes or escaped outside `unscale_uninit`
    /// casts (e.g., `core::slice::sort`'s local `MaybeUninit<[u8; N]>` scratch buffer).
    fn unscale_uninit_source_is_private(&self, src: ValueId, cast_res: Option<ValueId>) -> bool {
        let Some(cell) = self.trace_to_local_cell(src) else {
            return false;
        };
        // Ensure `cell` is never written to via `CellStore` with a foreign value or read/borrowed
        // for anything other than feeding an `unscale_uninit` cast (or `Drop` of `MaybeUninit`).
        let mut visited = BTreeSet::new();
        let mut queue: Vec<ValueId> = self.cell_reads[cell.0 as usize].clone();
        while let Some(v) = queue.pop() {
            if !visited.insert(v) {
                continue;
            }
            for site in &self.uses[v.0 as usize] {
                match site {
                    UseSite::EdgeArg(param) => queue.push(*param),
                    UseSite::Return | UseSite::OtherTerm => return false,
                    UseSite::Op(op) => match &op.kind {
                        OpKind::Copy(_) | OpKind::Move(_) => {
                            if let Some(r) = op.result {
                                queue.push(r);
                            }
                        }
                        OpKind::Cast { .. } if op.result == cast_res => {
                            // The `unscale_uninit` cast itself.
                        }
                        OpKind::Cast {
                            kind: CastKind::PtrToPtr | CastKind::PointerCoercion,
                            ..
                        } => {
                            if let Some(r) = op.result {
                                queue.push(r);
                            }
                        }
                        OpKind::BorrowView { path, .. } | OpKind::RawView { path, .. }
                            if !path.iter().any(|p| matches!(p, Proj::Deref)) =>
                        {
                            if let Some(r) = op.result {
                                queue.push(r);
                            }
                        }
                        OpKind::Drop {
                            glue: DropGlue::None,
                            ..
                        } => {}
                        _ => return false,
                    },
                }
            }
        }
        true
    }

    fn trace_to_local_cell(&self, start: ValueId) -> Option<CellId> {
        let mut cur = start;
        let mut seen = BTreeSet::new();
        while seen.insert(cur) {
            let op = self.def_map[cur.0 as usize]?;
            match &op.kind {
                OpKind::Copy(v) | OpKind::Move(v) | OpKind::Cast { v, .. } => cur = *v,
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    path,
                    ..
                }
                | OpKind::RawView {
                    root: Root::Cell(cell),
                    path,
                    ..
                } if !path.iter().any(|p| matches!(p, Proj::Deref)) => {
                    // Verify `cell` is locally allocated via `CellAlloc` (not a parameter cell).
                    let is_local_alloc = self.func.blocks.iter().any(|b| {
                        self.reached.contains(&b.id)
                            && b.ops
                                .iter()
                                .any(|o| matches!(o.kind, OpKind::CellAlloc(c) if c == *cell))
                    });
                    return is_local_alloc.then_some(*cell);
                }
                OpKind::BorrowView {
                    root: Root::Value(v),
                    path,
                    ..
                }
                | OpKind::RawView {
                    root: Root::Value(v),
                    path,
                    ..
                } if !path.iter().any(|p| matches!(p, Proj::Deref)) => cur = *v,
                _ => return None,
            }
        }
        None
    }
}

fn is_stdlib_alloc_wrapper(item: &LinkItem, std_crate_ids: &BTreeSet<u64>) -> bool {
    let path = item.debug_path.as_str();
    if path.starts_with("__rustc::")
        || path.contains("::__rust_alloc")
        || path.contains("::__rust_realloc")
    {
        return true;
    }
    let is_std_def = item.crate_name == "core"
        || item.crate_name == "alloc"
        || item
            .jir
            .body
            .as_ref()
            .is_some_and(|b| std_crate_ids.contains(&b.item.definition.crate_id));
    if !is_std_def {
        return false;
    }
    path.starts_with("core::alloc::")
        || path.starts_with("alloc::alloc::")
        || path.starts_with("alloc::Global::")
        || path.starts_with("<alloc::Global as ")
        || path.starts_with("<alloc::alloc::Global as ")
        || path.starts_with("alloc::raw_vec::")
        || path.starts_with("raw_vec::")
        || path.starts_with("alloc::boxed::")
        || path.starts_with("<alloc::boxed::")
        || path.starts_with("boxed::")
        || path.starts_with("<boxed::")
        || path.starts_with("alloc::rc::")
        || path.starts_with("<alloc::rc::")
        || path.starts_with("rc::")
        || path.starts_with("<rc::")
        || path.starts_with("alloc::sync::")
        || path.starts_with("<alloc::sync::")
        || path.starts_with("sync::")
        || path.starts_with("<sync::")
        || path.starts_with("alloc::vec::")
        || path.starts_with("<alloc::vec::")
        || path.starts_with("vec::")
        || path.starts_with("<vec::")
        || path.starts_with("alloc::string::")
        || path.starts_with("<alloc::string::")
        || path.starts_with("string::")
        || path.starts_with("<string::")
        || path.starts_with("alloc::slice::")
        || path.starts_with("slice::")
        || path.contains(" as alloc::slice::")
        || path.contains(" as slice::")
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
            if let Some(target) = items.get(name) {
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

/// Returns [`ScalarView::IntOrChar`] for wide integer/char scalars, [`ScalarView::Float`] for
/// `f32`/`f64`, and `None` for non-scalar types.
fn scalar_view_origin(table: &TypeTable, ty: TypeId) -> Option<ScalarView> {
    let peeled = table.peeled(ty);
    match value::kind(table, peeled) {
        Some(TypeKind::Integer {
            signed,
            bits: bits @ (16 | 32 | 64 | 128),
        }) => Some(ScalarView::IntOrChar(ScalarOrigin {
            bits: *bits,
            signed: *signed,
            is_char: false,
            size: u64::from(*bits / 8),
        })),
        Some(TypeKind::Char) => Some(ScalarView::IntOrChar(ScalarOrigin {
            bits: 32,
            signed: false,
            is_char: true,
            size: 4,
        })),
        Some(TypeKind::Float { bits: 32 | 64 }) => Some(ScalarView::Float),
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

/// Returns `Some((is_raw_bv, reason))` if the cast immediately requires byte-view machinery.
fn classify_cast(table: &TypeTable, op: &Op, hits: &mut CastHits) -> Option<(bool, &'static str)> {
    let OpKind::Cast { kind, v, from, to } = op.kind else {
        return None;
    };
    let result = op.result;
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
        hits.unscale_uninit_casts.push((v, result));
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

    if to_size == Some(1) && !table.is_indirect(to_p) {
        if let Some(es) = eff_from_size.filter(|&s| s > 1) {
            if is_byte_int(table, to_p) {
                if let Some(view) = scalar_view_origin(table, eff_from) {
                    if let Some(res) = result {
                        hits.scalar_scales.push((res, es, eff_from, view));
                        return None;
                    }
                    return Some((false, "scale_scalar_no_result"));
                }
            }
            if let Some(res) = result {
                hits.non_scalar_scales.push((res, es, eff_from));
            } else {
                return Some((true, "scale_no_result"));
            }
        }
    }

    None
}

fn is_std_adt(table: &TypeTable, ty: TypeId, std_crate_ids: &BTreeSet<u64>) -> bool {
    let Some(desc) = table.get(ty) else {
        return false;
    };
    if matches!(
        desc.meta.candidate,
        RepresentationCandidate::Vec
            | RepresentationCandidate::Box
            | RepresentationCandidate::String
    ) {
        return true;
    }
    matches!(&desc.kind, TypeKind::Adt { def, .. } if std_crate_ids.contains(&def.crate_id))
}

fn wide_vec_or_rawvec_elem_size(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    std_crate_ids: &BTreeSet<u64>,
) -> Option<u64> {
    let mut cur = table.peeled(ty);
    if let Some(p) = value::pointee_of(table, cur) {
        cur = table.peeled(p);
    }
    let desc = table.get(cur)?;
    let TypeKind::Adt { args, .. } = &desc.kind else {
        return None;
    };
    let is_vec_or_rawvec = matches!(desc.meta.candidate, RepresentationCandidate::Vec)
        || (is_std_adt(table, cur, std_crate_ids)
            && matches!(adt_short_name(table, interns, cur), "Vec" | "RawVec"));
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

fn is_byte_ptr_or_rawvec_inner(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    std_crate_ids: &BTreeSet<u64>,
) -> bool {
    let cur = table.peeled(ty);
    if is_byte_ptr(table, cur) {
        return true;
    }
    let has_byte_ptr_field = |adt_ty: TypeId| -> bool {
        let Some(TypeKind::Adt { variants, .. }) = table.get(adt_ty).map(|d| &d.kind) else {
            return false;
        };
        let Some(variant) = variants.first() else {
            return false;
        };
        variant.fields.iter().any(|f| {
            let fty = table.peeled(f.ty);
            if is_byte_ptr(table, fty) {
                return true;
            }
            if is_std_adt(table, fty, std_crate_ids)
                && adt_short_name(table, interns, fty) == "RawVecInner"
            {
                if let Some(TypeKind::Adt { variants: ivs, .. }) = table.get(fty).map(|d| &d.kind) {
                    return ivs.first().is_some_and(|v| {
                        v.fields
                            .iter()
                            .any(|ff| is_byte_ptr(table, table.peeled(ff.ty)))
                    });
                }
            }
            false
        })
    };
    (is_std_adt(table, ty, std_crate_ids)
        && matches!(adt_short_name(table, interns, ty), "RawVecInner" | "RawVec")
        && has_byte_ptr_field(ty))
        || (is_std_adt(table, cur, std_crate_ids)
            && matches!(
                adt_short_name(table, interns, cur),
                "RawVecInner" | "RawVec"
            )
            && has_byte_ptr_field(cur))
}

fn root_wide_vec_elem_size(
    func: &Func,
    table: &TypeTable,
    interns: &Interns,
    root: &Root,
    path: &[Proj],
    std_crate_ids: &BTreeSet<u64>,
) -> Option<u64> {
    let mut ty = func.root_ty(*root)?;
    if let Some(sz) = wide_vec_or_rawvec_elem_size(table, interns, ty, std_crate_ids) {
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
        if let Some(sz) = wide_vec_or_rawvec_elem_size(table, interns, ty, std_crate_ids) {
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

fn collect_def_map<'a>(func: &'a Func, reached: &BTreeSet<RBlockId>) -> Vec<Option<&'a Op>> {
    let mut map = vec![None; func.values.len()];
    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            if let Some(res) = op.result {
                map[res.0 as usize] = Some(op);
            }
        }
    }
    map
}

fn collect_zero_consts(func: &Func, reached: &BTreeSet<RBlockId>) -> Vec<bool> {
    let mut zeros = vec![false; func.values.len()];
    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            let Some(res) = op.result else { continue };
            match &op.kind {
                OpKind::Const(ConstTree::Scalar { bits: 0, .. }) => {
                    zeros[res.0 as usize] = true;
                }
                OpKind::Copy(v) | OpKind::Move(v) if zeros[v.0 as usize] => {
                    zeros[res.0 as usize] = true;
                }
                _ => {}
            }
        }
    }
    zeros
}

fn collect_unsigned_consts(
    func: &Func,
    ty_table: &TypeTable,
    reached: &BTreeSet<RBlockId>,
) -> Vec<Option<u128>> {
    let mut map = vec![None; func.values.len()];
    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            let Some(res) = op.result else { continue };
            let res_ty = func.values[res.0 as usize].ty;
            match &op.kind {
                OpKind::Const(ConstTree::Scalar { bits, .. })
                    if matches!(
                        value::kind(ty_table, ty_table.peeled(res_ty)),
                        Some(TypeKind::Integer { signed: false, .. })
                    ) =>
                {
                    map[res.0 as usize] = Some(*bits);
                }
                OpKind::Copy(v) | OpKind::Move(v) => {
                    if let Some(c) = map[v.0 as usize] {
                        map[res.0 as usize] = Some(c);
                    }
                }
                OpKind::Cast {
                    kind: CastKind::IntToInt,
                    v,
                    from,
                    to,
                } => {
                    if let (
                        Some(TypeKind::Integer {
                            signed: false,
                            bits: fb,
                        }),
                        Some(TypeKind::Integer {
                            signed: false,
                            bits: tb,
                        }),
                    ) = (
                        value::kind(ty_table, ty_table.peeled(*from)),
                        value::kind(ty_table, ty_table.peeled(*to)),
                    ) {
                        if tb >= fb {
                            if let Some(c) = map[v.0 as usize] {
                                map[res.0 as usize] = Some(c);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    map
}

fn collect_value_uses<'a>(
    func: &'a Func,
    reached: &BTreeSet<RBlockId>,
) -> (Vec<Vec<UseSite<'a>>>, Vec<Vec<ValueId>>) {
    let block_params: BTreeMap<RBlockId, &[ValueId]> = func
        .blocks
        .iter()
        .map(|b| (b.id, b.params.as_slice()))
        .collect();
    let mut map: Vec<Vec<UseSite<'a>>> = (0..func.values.len()).map(|_| Vec::new()).collect();
    let mut cell_reads: Vec<Vec<ValueId>> = vec![Vec::new(); func.cells.len()];

    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            for used in op.kind.uses() {
                map[used.0 as usize].push(UseSite::Op(op));
            }
            if let Some(res) = op.result {
                match &op.kind {
                    OpKind::CellLoad(cell) => {
                        cell_reads[cell.0 as usize].push(res);
                    }
                    OpKind::BorrowView {
                        root: Root::Cell(cell),
                        ..
                    }
                    | OpKind::RawView {
                        root: Root::Cell(cell),
                        ..
                    } => {
                        cell_reads[cell.0 as usize].push(res);
                    }
                    _ => {}
                }
            }
        }
        match &block.term {
            RTerm::Return(Some(v)) => {
                map[v.0 as usize].push(UseSite::Return);
            }
            RTerm::Throw(Some(v)) => {
                map[v.0 as usize].push(UseSite::OtherTerm);
            }
            RTerm::Return(None) | RTerm::Throw(None) | RTerm::Unreachable => {}
            RTerm::Switch { scrut, .. } => {
                map[scrut.0 as usize].push(UseSite::OtherTerm);
                for (target, args) in block.term.edges() {
                    if !reached.contains(&target) {
                        continue;
                    }
                    let params = block_params.get(&target).copied().unwrap_or(&[]);
                    for (idx, arg) in args.iter().enumerate() {
                        if let Some(&param) = params.get(idx) {
                            map[arg.0 as usize].push(UseSite::EdgeArg(param));
                        } else {
                            map[arg.0 as usize].push(UseSite::OtherTerm);
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
                            map[arg.0 as usize].push(UseSite::EdgeArg(param));
                        } else {
                            map[arg.0 as usize].push(UseSite::OtherTerm);
                        }
                    }
                }
            }
        }
    }
    (map, cell_reads)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OriginLattice {
    Bottom,
    Exact(ScalarOrigin),
    Top,
}

impl OriginLattice {
    fn meet(self, other: Self) -> Self {
        match (self, other) {
            (Self::Bottom, x) | (x, Self::Bottom) => x,
            (Self::Top, _) | (_, Self::Top) => Self::Top,
            (Self::Exact(a), Self::Exact(b)) if a == b => Self::Exact(a),
            (Self::Exact(_), Self::Exact(_)) => Self::Top,
        }
    }
}

/// Must-dataflow propagation of `ScalarOrigin` across simple value copies, block-param phis,
/// private cell stores/loads, and same-sign byte pointer casts.
///
/// Every non-derived value (function params, calls, ordinary `&[u8]` pointers, etc.) starts at
/// `Top`, and every block-param phi or cell takes the must-intersection (`meet`) over ALL
/// reachable incoming edges or stores. Any unknown or conflicting incoming path collapses the
/// target to `Top` (`None`).
fn propagate_scalar_origins(
    func: &Func,
    reached: &BTreeSet<RBlockId>,
    table: &TypeTable,
    scalar_scales: &[(ValueId, ScalarView)],
) -> Vec<Option<ScalarOrigin>> {
    if func.blocks.is_empty() {
        return vec![None; func.values.len()];
    }

    let entry_id = func.blocks[0].id;
    let block_params: BTreeMap<RBlockId, &[ValueId]> = func
        .blocks
        .iter()
        .map(|b| (b.id, b.params.as_slice()))
        .collect();

    let mut scale_map: BTreeMap<ValueId, OriginLattice> = BTreeMap::new();
    for &(val, view) in scalar_scales {
        let lat = view
            .int_or_char()
            .map_or(OriginLattice::Top, OriginLattice::Exact);
        scale_map
            .entry(val)
            .and_modify(|cur| *cur = cur.meet(lat))
            .or_insert(lat);
    }

    // Collect all reachable incoming edges per block parameter, reachable cell allocations/stores,
    // and cells whose address is taken mutably or raw (which may be written indirectly).
    let mut incoming: BTreeMap<ValueId, Vec<ValueId>> = BTreeMap::new();
    let mut alloc_cells: BTreeSet<CellId> = BTreeSet::new();
    let mut escaped_cells: BTreeSet<CellId> = BTreeSet::new();
    let mut cell_stores: BTreeMap<CellId, Vec<ValueId>> = BTreeMap::new();

    #[derive(Clone, Copy)]
    enum DerivedOp {
        CopyOf(ValueId),
        CellLoad(CellId),
    }
    let mut derived_ops: Vec<(ValueId, DerivedOp)> = Vec::new();

    for block in &func.blocks {
        if !reached.contains(&block.id) {
            continue;
        }
        for op in &block.ops {
            match &op.kind {
                OpKind::CellAlloc(cell) => {
                    alloc_cells.insert(*cell);
                }
                OpKind::CellStore { cell, v } => {
                    cell_stores.entry(*cell).or_default().push(*v);
                }
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    mutable: true,
                    ..
                }
                | OpKind::RawView {
                    root: Root::Cell(cell),
                    ..
                } => {
                    escaped_cells.insert(*cell);
                }
                OpKind::Copy(src) | OpKind::Move(src) => {
                    if let Some(res) = op.result {
                        if !scale_map.contains_key(&res) {
                            derived_ops.push((res, DerivedOp::CopyOf(*src)));
                        }
                    }
                }
                OpKind::Cast { v, from, to, .. } if same_byte_ptr_sign(table, *from, *to) => {
                    if let Some(res) = op.result {
                        if !scale_map.contains_key(&res) {
                            derived_ops.push((res, DerivedOp::CopyOf(*v)));
                        }
                    }
                }
                OpKind::CellLoad(cell) => {
                    if let Some(res) = op.result {
                        if !scale_map.contains_key(&res) {
                            derived_ops.push((res, DerivedOp::CellLoad(*cell)));
                        }
                    }
                }
                _ => {}
            }
        }
        for (target, args) in block.term.edges() {
            if !reached.contains(&target) {
                continue;
            }
            let params = block_params.get(&target).copied().unwrap_or(&[]);
            for (idx, &param) in params.iter().enumerate() {
                if let Some(&arg) = args.get(idx) {
                    incoming.entry(param).or_default().push(arg);
                }
            }
        }
    }

    let mut val_state = vec![OriginLattice::Top; func.values.len()];
    for (&v, &lat) in &scale_map {
        if let Some(slot) = val_state.get_mut(v.0 as usize) {
            *slot = lat;
        }
    }
    for &(res, _) in &derived_ops {
        if let Some(slot) = val_state.get_mut(res.0 as usize) {
            *slot = OriginLattice::Bottom;
        }
    }
    let mut phi_params: Vec<ValueId> = Vec::new();
    for block in &func.blocks {
        if !reached.contains(&block.id) || block.id == entry_id {
            continue;
        }
        for &param in &block.params {
            if !scale_map.contains_key(&param)
                && incoming.get(&param).is_some_and(|edges| !edges.is_empty())
            {
                if let Some(slot) = val_state.get_mut(param.0 as usize) {
                    *slot = OriginLattice::Bottom;
                    phi_params.push(param);
                }
            }
        }
    }

    let mut cell_state = vec![OriginLattice::Top; func.cells.len()];
    let mut tracked_cells: Vec<CellId> = Vec::new();
    for &cell in &alloc_cells {
        if !escaped_cells.contains(&cell) && cell_stores.get(&cell).is_some_and(|s| !s.is_empty()) {
            if let Some(slot) = cell_state.get_mut(cell.0 as usize) {
                *slot = OriginLattice::Bottom;
                tracked_cells.push(cell);
            }
        }
    }

    loop {
        let mut changed = false;

        for &param in &phi_params {
            let mut joined = OriginLattice::Bottom;
            if let Some(edges) = incoming.get(&param) {
                for &arg in edges {
                    let arg_lat = val_state
                        .get(arg.0 as usize)
                        .copied()
                        .unwrap_or(OriginLattice::Top);
                    joined = joined.meet(arg_lat);
                }
            } else {
                joined = OriginLattice::Top;
            }
            let slot = &mut val_state[param.0 as usize];
            let next = slot.meet(joined);
            if next != *slot {
                *slot = next;
                changed = true;
            }
        }

        for &cell in &tracked_cells {
            let mut joined = OriginLattice::Bottom;
            if let Some(stores) = cell_stores.get(&cell) {
                for &src in stores {
                    let src_lat = val_state
                        .get(src.0 as usize)
                        .copied()
                        .unwrap_or(OriginLattice::Top);
                    joined = joined.meet(src_lat);
                }
            } else {
                joined = OriginLattice::Top;
            }
            let slot = &mut cell_state[cell.0 as usize];
            let next = slot.meet(joined);
            if next != *slot {
                *slot = next;
                changed = true;
            }
        }

        for &(res, dop) in &derived_ops {
            let src_lat = match dop {
                DerivedOp::CopyOf(src) => val_state
                    .get(src.0 as usize)
                    .copied()
                    .unwrap_or(OriginLattice::Top),
                DerivedOp::CellLoad(cell) => cell_state
                    .get(cell.0 as usize)
                    .copied()
                    .unwrap_or(OriginLattice::Top),
            };
            let slot = &mut val_state[res.0 as usize];
            let next = slot.meet(src_lat);
            if next != *slot {
                *slot = next;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    val_state
        .into_iter()
        .map(|lat| match lat {
            OriginLattice::Exact(o) => Some(o),
            OriginLattice::Bottom | OriginLattice::Top => None,
        })
        .collect()
}

impl<'a> ByteViewFuncCx<'a> {
    /// Returns `true` if unsigned integer value `val` is statically proven to be a multiple of
    /// `elem_size` (which is always a power of 2 in `{2, 4, 8, 16}` for wide scalars).
    fn is_multiple_of_elem_size(&self, val: ValueId, elem_size: u64) -> bool {
        if elem_size <= 1 || !elem_size.is_power_of_two() {
            return false;
        }
        let mut seen = BTreeSet::new();
        self.is_multiple_of_rec(val, elem_size, &mut seen)
    }

    fn is_multiple_of_rec(
        &self,
        val: ValueId,
        elem_size: u64,
        seen: &mut BTreeSet<ValueId>,
    ) -> bool {
        if !seen.insert(val) {
            return false;
        }
        if let Some(c) = self.u_consts[val.0 as usize] {
            return c % u128::from(elem_size) == 0;
        }
        let Some(op) = self.def_map[val.0 as usize] else {
            return false;
        };
        match &op.kind {
            OpKind::Copy(v) | OpKind::Move(v) => self.is_multiple_of_rec(*v, elem_size, seen),
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v,
                from,
                to,
            } => {
                if let (
                    Some(TypeKind::Integer {
                        signed: false,
                        bits: fb,
                    }),
                    Some(TypeKind::Integer {
                        signed: false,
                        bits: tb,
                    }),
                ) = (
                    value::kind(self.table, self.table.peeled(*from)),
                    value::kind(self.table, self.table.peeled(*to)),
                ) {
                    if tb >= fb {
                        return self.is_multiple_of_rec(*v, elem_size, seen);
                    }
                }
                false
            }
            OpKind::Binary { op: bop, l, r, ty } => {
                let Some(TypeKind::Integer {
                    signed: false,
                    bits,
                }) = value::kind(self.table, self.table.peeled(*ty))
                else {
                    return false;
                };
                match bop {
                    BinaryOp::Mul | BinaryOp::MulUnchecked => {
                        self.is_multiple_of_rec(*l, elem_size, seen)
                            || self.is_multiple_of_rec(*r, elem_size, seen)
                    }
                    BinaryOp::Shl | BinaryOp::ShlUnchecked => self.u_consts[r.0 as usize]
                        .is_some_and(|s| {
                            s >= u128::from(elem_size.trailing_zeros()) && s < u128::from(*bits)
                        }),
                    BinaryOp::Add
                    | BinaryOp::AddUnchecked
                    | BinaryOp::Sub
                    | BinaryOp::SubUnchecked => {
                        self.is_multiple_of_rec(*l, elem_size, seen)
                            && self.is_multiple_of_rec(*r, elem_size, seen)
                    }
                    _ => false,
                }
            }
            OpKind::Field { base, field: 0, .. } => {
                if let Some(base_op) = self.def_map[base.0 as usize] {
                    if let OpKind::Binary {
                        op: BinaryOp::MulWithOverflow,
                        l,
                        r,
                        ty,
                    } = &base_op.kind
                    {
                        if matches!(
                            value::kind(self.table, self.table.peeled(*ty)),
                            Some(TypeKind::Integer { signed: false, .. })
                        ) {
                            return self.is_multiple_of_rec(*l, elem_size, seen)
                                || self.is_multiple_of_rec(*r, elem_size, seen);
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// True if every use of `val` (the `i32` result of `compare_bytes`) is `val == 0` or `val != 0`.
    fn only_used_for_zero_equality(&self, val: ValueId) -> bool {
        let mut visited = BTreeSet::new();
        let mut queue = vec![val];
        while let Some(v) = queue.pop() {
            if !visited.insert(v) {
                continue;
            }
            for site in &self.uses[v.0 as usize] {
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
                        } if (*l == v && self.zero_consts[r.0 as usize])
                            || (*r == v && self.zero_consts[l.0 as usize]) => {}
                        _ => return false,
                    },
                }
            }
        }
        true
    }
}

enum CallEffect {
    /// Call consumes pointer safely without dereferencing or returning a tainted pointer.
    Sink,
    /// Call passes the pointer through into its return value (taint MUST propagate to `op.result`).
    PropagateToResult,
    /// Call may dereference or leak the pointer.
    Escape,
}

fn is_safe_std_drop_path(dp: &str) -> bool {
    dp.starts_with("core::ptr::drop_in_place<alloc::raw_vec::RawVec")
        || dp.starts_with("core::ptr::drop_in_place<alloc::vec::Vec<")
        || dp.starts_with("core::ptr::drop_in_place<alloc::vec::into_iter::IntoIter<")
        || dp.starts_with("core::ptr::drop_in_place<alloc::boxed::Box<")
}

fn is_std_drop_container(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    std_crate_ids: &BTreeSet<u64>,
) -> bool {
    for candidate in [ty, table.peeled(ty)] {
        if !is_std_adt(table, candidate, std_crate_ids) {
            continue;
        }
        if matches!(
            adt_short_name(table, interns, candidate),
            "RawVecInner" | "RawVec" | "Vec" | "IntoIter" | "Box" | "DropGuard"
        ) {
            return true;
        }
        if let Some(desc) = table.get(candidate) {
            if matches!(
                desc.meta.candidate,
                RepresentationCandidate::Vec | RepresentationCandidate::Box
            ) {
                return true;
            }
        }
    }
    false
}

fn is_safe_std_drop(
    glue: &DropGlue,
    dropped_ty: TypeId,
    table: &TypeTable,
    interns: &Interns,
    items: &BTreeMap<JsName, LinkItem>,
    std_crate_ids: &BTreeSet<u64>,
) -> bool {
    match glue {
        DropGlue::None => true,
        DropGlue::Dynamic => false,
        DropGlue::Direct { link_name, .. } => {
            if is_std_drop_container(table, interns, dropped_ty, std_crate_ids) {
                return true;
            }
            let name = interns.str(*link_name).unwrap_or("");
            items
                .get(name)
                .is_some_and(|t| is_safe_std_drop_path(t.debug_path.as_str()))
        }
    }
}

fn is_pure_metadata_ret(table: &TypeTable, ty: TypeId) -> bool {
    let peeled = table.peeled(ty);
    table.is_zst(peeled)
        || matches!(
            value::kind(table, peeled),
            Some(TypeKind::Bool | TypeKind::Integer { .. })
        )
}

impl<'a> ByteViewFuncCx<'a> {
    fn classify_stdlib_ptr_call(
        &self,
        callee: &Callee,
        result: Option<ValueId>,
        elem_size: Option<u64>,
        expected_origin: Option<ScalarOrigin>,
    ) -> CallEffect {
        match callee {
            Callee::Nop => CallEffect::Sink,
            Callee::Runtime { member, .. } => match self.interns.str(*member).unwrap_or("") {
                "dealloc" | "ptr_is_null" => CallEffect::Sink,
                "realloc" => CallEffect::PropagateToResult,
                m => {
                    if self.debug {
                        eprintln!("[bv]   escape runtime callee: {m}");
                    }
                    CallEffect::Escape
                }
            },
            Callee::Foreign { symbol, .. } => {
                let sym = self.interns.str(*symbol).unwrap_or("");
                if sym.contains("__rust_dealloc") {
                    CallEffect::Sink
                } else if sym.contains("__rust_realloc") {
                    CallEffect::PropagateToResult
                } else {
                    if self.debug {
                        eprintln!("[bv]   escape foreign callee: {sym}");
                    }
                    CallEffect::Escape
                }
            }
            Callee::Direct { link_name, .. } => {
                let name = self.interns.str(*link_name).unwrap_or("");
                if name.contains("__rust_dealloc") {
                    return CallEffect::Sink;
                }
                if name.contains("__rust_realloc") {
                    return CallEffect::PropagateToResult;
                }
                let Some(target) = self.items.get(name) else {
                    if self.debug {
                        eprintln!("[bv]   escape unknown direct callee: {name}");
                    }
                    return CallEffect::Escape;
                };
                let dp = target.debug_path.as_str();
                let target_is_std =
                    target.crate_name == "core"
                        || target.crate_name == "alloc"
                        || target.jir.body.as_ref().is_some_and(|b| {
                            self.std_crate_ids.contains(&b.item.definition.crate_id)
                        });
                // True sinks (free memory or query nullness/metadata without returning a pointer).
                if dp.starts_with("alloc::alloc::Global::deallocate")
                    || dp.starts_with("alloc::Global::deallocate")
                    || dp
                        .starts_with("<alloc::alloc::Global as core::alloc::Allocator>::deallocate")
                    || dp.starts_with("<alloc::Global as core::alloc::Allocator>::deallocate")
                    || dp.starts_with("alloc::alloc::dealloc")
                    || dp.starts_with("core::ptr::const_ptr::<impl *const T>::is_null")
                    || dp.starts_with("core::ptr::mut_ptr::<impl *mut T>::is_null")
                    || is_safe_std_drop_path(dp)
                    || (target_is_std
                        && (dp.ends_with("::deallocate")
                            || dp.ends_with("::deallocate_managed")
                            || dp.ends_with("::dealloc_elems")
                            || dp.contains("rcgjs_typed_dealloc")))
                    || matches!(
                        target.jir.body.as_ref().map(|b| b.semantic),
                        Some(rcgjs_object::jir::semantic::BodySemantic::Standard(
                            rcgjs_object::jir::semantic::StandardOperation::TypedDealloc
                                | rcgjs_object::jir::semantic::StandardOperation::TypedDeallocRaw
                        ))
                    )
                {
                    return CallEffect::Sink;
                }
                // Realloc propagates the pointer (possibly scaled) to its result.
                if dp.starts_with("alloc::alloc::realloc")
                    || dp.starts_with("alloc::alloc::Global::grow_impl")
                    || dp.starts_with("alloc::Global::grow_impl")
                    || dp.starts_with("alloc::alloc::Global::shrink_impl")
                    || dp.starts_with("alloc::Global::shrink_impl")
                    || (target_is_std && dp.ends_with("::realloc_elems"))
                    || matches!(
                        target.jir.body.as_ref().map(|b| b.semantic),
                        Some(rcgjs_object::jir::semantic::BodySemantic::Standard(
                            rcgjs_object::jir::semantic::StandardOperation::TypedRealloc
                        ))
                    )
                {
                    return CallEffect::PropagateToResult;
                }
                // NonNull / Unique constructors & pointer accessors ONLY (never read/write/as_ref!).
                if target_is_std
                    && (dp.starts_with("core::ptr::non_null::NonNull")
                        || dp.starts_with("ptr::non_null::NonNull"))
                {
                    if dp.ends_with("::new_unchecked")
                        || dp.ends_with("::new")
                        || dp.ends_with("::as_ptr")
                    {
                        return CallEffect::PropagateToResult;
                    }
                    if dp.ends_with("::cast") || dp.contains("::cast::<") {
                        return classify_wrapper_cast_result(
                            self.func,
                            self.table,
                            result,
                            elem_size,
                            expected_origin,
                        );
                    }
                    if self.debug {
                        eprintln!("[bv]   escape NonNull callee: {dp}");
                    }
                    return CallEffect::Escape;
                }
                if target_is_std
                    && (dp.starts_with("core::ptr::unique::Unique")
                        || dp.starts_with("ptr::unique::Unique"))
                {
                    if dp.ends_with("::new_unchecked")
                        || dp.ends_with("::new")
                        || dp.ends_with("::as_ptr")
                        || dp.ends_with("::as_non_null_ptr")
                    {
                        return CallEffect::PropagateToResult;
                    }
                    if dp.ends_with("::cast") || dp.contains("::cast::<") {
                        return classify_wrapper_cast_result(
                            self.func,
                            self.table,
                            result,
                            elem_size,
                            expected_origin,
                        );
                    }
                    if self.debug {
                        eprintln!("[bv]   escape Unique callee: {dp}");
                    }
                    return CallEffect::Escape;
                }
                // `alloc::raw_vec` internal plumbing: deallocate/reserve/capacity/grow/etc.
                if target_is_std
                    && (dp.starts_with("alloc::raw_vec::")
                        || dp.starts_with("raw_vec::RawVecInner"))
                {
                    if let Some(res) = result {
                        let res_ty = self.func.values[res.0 as usize].ty;
                        if is_pure_metadata_ret(self.table, res_ty) {
                            return CallEffect::Sink;
                        }
                        // If `RawVec::ptr` / `non_null` casts back to the exact `T` element type/size,
                        // it is equivalent to `unscale` back to `*mut T`.
                        if matches_unscale_target(
                            self.table,
                            self.table.peeled(res_ty),
                            elem_size,
                            expected_origin,
                        ) {
                            return CallEffect::Sink;
                        }
                        return CallEffect::PropagateToResult;
                    }
                    return CallEffect::Sink;
                }
                if self.debug {
                    eprintln!("[bv]   escape direct callee: {dp} ({name})");
                }
                CallEffect::Escape
            }
            _ => CallEffect::Escape,
        }
    }
}

fn matches_unscale_target(
    table: &TypeTable,
    res_ty: TypeId,
    elem_size: Option<u64>,
    expected_origin: Option<ScalarOrigin>,
) -> bool {
    let Some(p) = value::pointee_of(table, res_ty) else {
        return false;
    };
    if let Some(orig) = expected_origin {
        return scalar_view_origin(table, p) == Some(ScalarView::IntOrChar(orig));
    }
    elem_size.is_some_and(|sz| value::size_of(table, p) == Some(sz))
}

fn classify_wrapper_cast_result(
    func: &Func,
    table: &TypeTable,
    result: Option<ValueId>,
    elem_size: Option<u64>,
    expected_origin: Option<ScalarOrigin>,
) -> CallEffect {
    let Some(res) = result else {
        return CallEffect::Sink;
    };
    let res_ty = table.peeled(func.values[res.0 as usize].ty);
    if is_byte_ptr(table, res_ty) {
        return CallEffect::PropagateToResult;
    }
    if matches_unscale_target(table, res_ty, elem_size, expected_origin) {
        return CallEffect::Sink;
    }
    CallEffect::Escape
}

fn is_wrapper_aggregate(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    std_crate_ids: &BTreeSet<u64>,
) -> bool {
    for candidate in [ty, table.peeled(ty)] {
        if !is_std_adt(table, candidate, std_crate_ids) {
            continue;
        }
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
                RepresentationCandidate::Vec | RepresentationCandidate::Box
            ) {
                return true;
            }
            if let TypeKind::Adt { variants, .. } = &desc.kind {
                if variants.len() == 2
                    && variants.iter().any(|v| interns.str(v.name) == Some("Some"))
                    && variants.iter().any(|v| interns.str(v.name) == Some("None"))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// True if `ty` is an opaque **wide** stdlib container (`Vec<T>`, `RawVec<T>`, `Box<T>`,
/// `IntoIter<T>` with `size_of::<T>() > 1`) that may safely cross a function return boundary
/// because any caller unpacking its internal buffer pointer is guarded by Rule 5.
///
/// 1-byte containers (`Vec<u8>`, `Box<u8>`) and element-erased `RawVecInner` are intentionally
/// rejected so a scaled wide pointer can never escape disguised as a 1-byte container.
fn is_returnable_container(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    std_crate_ids: &BTreeSet<u64>,
) -> bool {
    for candidate in [ty, table.peeled(ty)] {
        if !is_std_adt(table, candidate, std_crate_ids) {
            continue;
        }
        let Some(desc) = table.get(candidate) else {
            continue;
        };
        let TypeKind::Adt { args, .. } = &desc.kind else {
            continue;
        };
        let is_container = matches!(
            desc.meta.candidate,
            RepresentationCandidate::Vec | RepresentationCandidate::Box
        ) || matches!(
            adt_short_name(table, interns, candidate),
            "RawVec" | "Vec" | "IntoIter" | "Box"
        );
        if is_container {
            if let Some(&elem) = args.first() {
                if value::size_of(table, elem).is_some_and(|s| s > 1) {
                    return true;
                }
            }
        }
    }
    false
}

/// True if `ty` is a wide stdlib `Vec<T>` or `RawVec<T>` whose element type matches
/// `(elem_size, eff_from, expected_origin)`. Sealing a scaled byte pointer into such a container
/// outside `in_stdlib_alloc` is safe because Rule 5 (`wide_rawvec_byte_ptrs`) guards any
/// later extraction of its internal `RawVecInner` byte pointer, and `classify_cast` guards any
/// later `*const T -> *const u8` cast.
fn is_sealed_wide_vec_container(
    table: &TypeTable,
    interns: &Interns,
    ty: TypeId,
    std_crate_ids: &BTreeSet<u64>,
    elem_size: u64,
    eff_from: TypeId,
    expected_origin: Option<ScalarOrigin>,
) -> bool {
    if elem_size <= 1 {
        return false;
    }
    let mut cur = table.peeled(ty);
    if let Some(p) = value::pointee_of(table, cur) {
        cur = table.peeled(p);
    }
    let Some(desc) = table.get(cur) else {
        return false;
    };
    let TypeKind::Adt { args, .. } = &desc.kind else {
        return false;
    };
    let is_vec_or_rawvec = matches!(desc.meta.candidate, RepresentationCandidate::Vec)
        || (is_std_adt(table, cur, std_crate_ids)
            && matches!(adt_short_name(table, interns, cur), "Vec" | "RawVec"));
    if !is_vec_or_rawvec {
        return false;
    }
    let Some(&elem) = args.first() else {
        return false;
    };
    if let Some(orig) = expected_origin {
        return scalar_view_origin(table, elem) == Some(ScalarView::IntOrChar(orig));
    }
    if let Some(TypeKind::Float { bits }) = value::kind(table, eff_from) {
        return matches!(
            value::kind(table, table.peeled(elem)),
            Some(TypeKind::Float { bits: eb }) if eb == bits
        );
    }
    scalar_view_origin(table, elem).is_none() && value::size_of(table, elem) == Some(elem_size)
}

impl<'a> ByteViewFuncCx<'a> {
    /// Checks whether a `scale` / `scale_scalar` result escapes outside wide stdlib container
    /// plumbing (`RawVec`/`Vec`/`Box`/`IntoIter`/`dealloc`/`realloc`), or (when `origin` is
    /// `Some(ScalarOrigin)`) same-type whole-element `compare_bytes(a, b, n) == 0 / != 0`.
    /// Returns `Some(reason)` if it escapes.
    fn scale_value_escapes(
        &self,
        start: ValueId,
        elem_size: u64,
        eff_from: TypeId,
        origin: Option<ScalarOrigin>,
    ) -> Option<&'static str> {
        let mut visited = BTreeSet::new();
        let mut queue = vec![start];
        while let Some(v) = queue.pop() {
            if !visited.insert(v) {
                continue;
            }
            for site in &self.uses[v.0 as usize] {
                match site {
                    UseSite::EdgeArg(param) => queue.push(*param),
                    UseSite::Return => {
                        let ty = self.func.values[v.0 as usize].ty;
                        if !is_returnable_container(
                            self.table,
                            self.interns,
                            ty,
                            self.std_crate_ids,
                        ) {
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
                            queue.extend(self.cell_reads[cell.0 as usize].iter().copied());
                        }
                        // Mutability cast `*mut u8 -> *const u8` (same signedness byte ptr).
                        OpKind::Cast { from, to, .. }
                            if same_byte_ptr_sign(self.table, *from, *to) =>
                        {
                            if let Some(res) = op.result {
                                queue.push(res);
                            }
                        }
                        OpKind::Aggregate { ty, .. }
                            if !self.in_stdlib_alloc
                                && is_sealed_wide_vec_container(
                                    self.table,
                                    self.interns,
                                    *ty,
                                    self.std_crate_ids,
                                    elem_size,
                                    eff_from,
                                    origin,
                                ) =>
                        {
                            // Sealed inside a matching wide `Vec<T>` / `RawVec<T>`; Rule 5 guards
                            // any subsequent extraction of its internal `RawVecInner` byte pointer.
                        }
                        OpKind::Aggregate { ty, .. }
                            if is_wrapper_aggregate(
                                self.table,
                                self.interns,
                                *ty,
                                self.std_crate_ids,
                            ) =>
                        {
                            if let Some(res) = op.result {
                                queue.push(res);
                            }
                        }
                        OpKind::Field { .. } => {
                            if let Some(res) = op.result {
                                let res_ty = self.func.values[res.0 as usize].ty;
                                if !is_pure_metadata_ret(self.table, res_ty) {
                                    queue.push(res);
                                }
                            }
                        }
                        OpKind::Call { callee, .. } => {
                            match self.classify_stdlib_ptr_call(
                                callee,
                                op.result,
                                origin.map(|o| o.size),
                                origin,
                            ) {
                                CallEffect::Sink => {}
                                CallEffect::PropagateToResult => {
                                    if let Some(res) = op.result {
                                        queue.push(res);
                                    }
                                }
                                CallEffect::Escape => return Some("call"),
                            }
                        }
                        OpKind::Intrinsic { name, args, .. }
                            if origin.is_some()
                                && self.interns.str(*name) == Some("compare_bytes") =>
                        {
                            let Some(res) = op.result else {
                                return Some("compare_bytes_no_res");
                            };
                            if args.len() < 3 {
                                return Some("compare_bytes_args");
                            }
                            let (Some(o0), Some(o1)) = (
                                self.scalar_origins[args[0].0 as usize],
                                self.scalar_origins[args[1].0 as usize],
                            ) else {
                                return Some("compare_bytes_origin");
                            };
                            if o0 != o1 {
                                return Some("compare_bytes_type_mismatch");
                            }
                            if !self.is_multiple_of_elem_size(args[2], o0.size) {
                                return Some("compare_bytes_unaligned_count");
                            }
                            if !self.only_used_for_zero_equality(res) {
                                return Some("compare_bytes_ord");
                            }
                        }
                        OpKind::Drop { glue, .. } => {
                            let dropped_ty = self.func.values[v.0 as usize].ty;
                            if !is_safe_std_drop(
                                glue,
                                dropped_ty,
                                self.table,
                                self.interns,
                                self.items,
                                self.std_crate_ids,
                            ) {
                                return Some("drop");
                            }
                        }
                        OpKind::Aggregate { .. } => return Some("aggregate"),
                        OpKind::Cast { .. } => return Some("cast"),
                        OpKind::Intrinsic { .. } => return Some("intrinsic"),
                        OpKind::SetField { .. } => return Some("set_field"),
                        OpKind::StoreThrough { .. } => return Some("store_through"),
                        OpKind::BorrowView { root, path, .. }
                        | OpKind::RawView { root, path, .. } => {
                            // Taking a whole-container reference/pointer (`&Vec<T>`, `&mut Vec<T>`,
                            // `&RawVec<T>`) does not expose its internal raw byte pointer directly;
                            // any consumer that unpacks the buffer pointer is guarded by Rule 5.
                            if path.is_empty()
                                && matches!(root, Root::Value(rv) if *rv == v)
                                && wide_vec_or_rawvec_elem_size(
                                    self.table,
                                    self.interns,
                                    self.func.values[v.0 as usize].ty,
                                    self.std_crate_ids,
                                )
                                .is_some()
                            {
                                continue;
                            }
                            return Some("view");
                        }
                        _ => return Some("other_op"),
                    },
                }
            }
        }
        None
    }

    /// Checks whether a byte pointer projected out of a wide `Vec<T>` / `RawVec<T>` escapes
    /// into any operation other than re-casting back to `T` (`unscale`/`unscale_scalar`),
    /// rebuilding `RawVec`/`NonNull`, or calling `RawVecInner`/`dealloc`/`realloc`/`ptr_is_null`.
    fn rawvec_byte_ptr_escapes(&self, start: ValueId, elem_size: u64) -> bool {
        let mut visited = BTreeSet::new();
        let mut queue = vec![start];
        while let Some(v) = queue.pop() {
            if !visited.insert(v) {
                continue;
            }
            for site in &self.uses[v.0 as usize] {
                match site {
                    UseSite::EdgeArg(param) => queue.push(*param),
                    UseSite::Return => {
                        let ty = self.func.values[v.0 as usize].ty;
                        if !is_returnable_container(
                            self.table,
                            self.interns,
                            ty,
                            self.std_crate_ids,
                        ) {
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
                            queue.extend(self.cell_reads[cell.0 as usize].iter().copied());
                        }
                        OpKind::Field { .. } => {
                            if let Some(res) = op.result {
                                let res_ty = self.func.values[res.0 as usize].ty;
                                if !is_pure_metadata_ret(self.table, res_ty) {
                                    queue.push(res);
                                }
                            }
                        }
                        OpKind::BorrowView { root, path, .. }
                        | OpKind::RawView { root, path, .. } => {
                            if path.iter().any(|p| matches!(p, Proj::Deref)) {
                                return true;
                            }
                            if path.is_empty()
                                && matches!(root, Root::Value(rv) if *rv == v)
                                && wide_vec_or_rawvec_elem_size(
                                    self.table,
                                    self.interns,
                                    self.func.values[v.0 as usize].ty,
                                    self.std_crate_ids,
                                )
                                .is_some()
                            {
                                continue;
                            }
                            if let Some(res) = op.result {
                                queue.push(res);
                            } else {
                                return true;
                            }
                        }
                        OpKind::Aggregate { ty, .. }
                            if is_wrapper_aggregate(
                                self.table,
                                self.interns,
                                *ty,
                                self.std_crate_ids,
                            ) =>
                        {
                            if let Some(res) = op.result {
                                queue.push(res);
                            }
                        }
                        OpKind::Cast { from, to, .. } => {
                            if same_byte_ptr_sign(self.table, *from, *to) {
                                if let Some(res) = op.result {
                                    queue.push(res);
                                }
                                continue;
                            }
                            let ok = value::pointee_of(self.table, *to)
                                .and_then(|p| value::size_of(self.table, p))
                                .is_some_and(|s| s == elem_size);
                            if !ok {
                                return true;
                            }
                        }
                        OpKind::Call { callee, .. } => {
                            match self.classify_stdlib_ptr_call(
                                callee,
                                op.result,
                                Some(elem_size),
                                None,
                            ) {
                                CallEffect::Sink => {}
                                CallEffect::PropagateToResult => {
                                    if let Some(res) = op.result {
                                        queue.push(res);
                                    }
                                }
                                CallEffect::Escape => return true,
                            }
                        }
                        OpKind::Drop { glue, .. } => {
                            let dropped_ty = self.func.values[v.0 as usize].ty;
                            if !is_safe_std_drop(
                                glue,
                                dropped_ty,
                                self.table,
                                self.interns,
                                self.items,
                                self.std_crate_ids,
                            ) {
                                return true;
                            }
                        }
                        _ => return true,
                    },
                }
            }
        }
        false
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use rcgjs_object::jir::origin::OriginId;
    use rcgjs_object::jir::repr::{Carrier, RBlock, ValueDef};
    use rcgjs_object::jir::semantic::{AbiSignature, DefinitionId, ItemId, Signature};
    use rcgjs_object::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, NominalForm, TypeDescriptor, TypeMeta,
        VariantDescriptor,
    };

    fn make_desc(id: u128, kind: TypeKind, size: u64, int: Option<(bool, u32)>) -> TypeDescriptor {
        let id = TypeId(id);
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                align: Some(size.max(1)),
                sized: true,
                zst: size == 0,
                ..LayoutFacts::default()
            },
            js: JsFacts {
                int: int.map(|(signed, bits)| IntFacts {
                    signed,
                    bits,
                    repr: if bits > 32 {
                        IntRepr::BigInt
                    } else {
                        IntRepr::Number
                    },
                }),
                is_indirect: false,
                needs_clone: false,
                peeled: id,
                transparent: None,
                crosses_transparent_union: false,
                enum_repr: None,
                is_primitive_enum: false,
                unsized_tail: None,
                byte_element: matches!(int, Some((_, 8))),
                pointer_form: None,
                fields: Vec::new(),
            },
            meta: TypeMeta::default(),
        }
    }

    fn make_func(values: Vec<TypeId>, blocks: Vec<RBlock>) -> Func {
        Func {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 999,
                    local_hash: 1,
                },
                instantiation: 1,
            },
            sig: Signature {
                arguments: Vec::new(),
                result: TypeId(0),
                abi: AbiSignature::default(),
            },
            values: values
                .into_iter()
                .map(|ty| ValueDef {
                    ty,
                    carrier: Carrier::Baseline,
                    origin: OriginId(0),
                })
                .collect(),
            cells: Vec::new(),
            params: Vec::new(),
            blocks,
        }
    }

    fn op(res: Option<u32>, kind: OpKind) -> Op {
        Op {
            result: res.map(ValueId),
            kind,
            origin: OriginId(0),
        }
    }

    #[test]
    fn reachable_blocks_does_not_misfold_int_not_signed_cmp_or_float_cmp() {
        let mut table = TypeTable::new();
        let t_bool = TypeId(0);
        let t_u8 = TypeId(1);
        let t_i32 = TypeId(2);
        let t_f64 = TypeId(3);
        table.insert(make_desc(0, TypeKind::Bool, 1, None));
        table.insert(make_desc(
            1,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            1,
            Some((false, 8)),
        ));
        table.insert(make_desc(
            2,
            TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            4,
            Some((true, 32)),
        ));
        table.insert(make_desc(3, TypeKind::Float { bits: 64 }, 8, None));

        // Case 1: `!0u8 == 255u8` must NOT fold `!0u8` to `1` and kill the true branch (bb1).
        let f_not = make_func(
            vec![t_u8, t_u8, t_u8, t_bool],
            vec![
                RBlock {
                    id: RBlockId(0),
                    params: vec![],
                    ops: vec![
                        op(
                            Some(0),
                            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
                        ),
                        op(
                            Some(1),
                            OpKind::Unary {
                                op: UnaryOp::Not,
                                v: ValueId(0),
                                ty: t_u8,
                            },
                        ),
                        op(
                            Some(2),
                            OpKind::Const(ConstTree::Scalar {
                                bits: 255,
                                bytes: 1,
                            }),
                        ),
                        op(
                            Some(3),
                            OpKind::Binary {
                                op: BinaryOp::Eq,
                                l: ValueId(1),
                                r: ValueId(2),
                                ty: t_u8,
                            },
                        ),
                    ],
                    term: RTerm::Switch {
                        scrut: ValueId(3),
                        scrut_ty: t_bool,
                        cases: vec![(0, RBlockId(2), vec![])],
                        default: (RBlockId(1), vec![]),
                    },
                },
                RBlock {
                    id: RBlockId(1),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Return(None),
                },
                RBlock {
                    id: RBlockId(2),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Return(None),
                },
            ],
        );
        let r_not = reachable_blocks(&f_not, &table);
        assert!(
            r_not.contains(&RBlockId(1)),
            "!0u8 branch must remain reachable"
        );
        assert!(r_not.contains(&RBlockId(2)));

        // Case 2: `-1i32 < 0i32` must NOT fold unsigned (0xffffffff < 0 => false) and kill bb1.
        let f_signed = make_func(
            vec![t_i32, t_i32, t_bool],
            vec![
                RBlock {
                    id: RBlockId(0),
                    params: vec![],
                    ops: vec![
                        op(
                            Some(0),
                            OpKind::Const(ConstTree::Scalar {
                                bits: 0xffff_ffff,
                                bytes: 4,
                            }),
                        ),
                        op(
                            Some(1),
                            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
                        ),
                        op(
                            Some(2),
                            OpKind::Binary {
                                op: BinaryOp::Lt,
                                l: ValueId(0),
                                r: ValueId(1),
                                ty: t_i32,
                            },
                        ),
                    ],
                    term: RTerm::Switch {
                        scrut: ValueId(2),
                        scrut_ty: t_bool,
                        cases: vec![(0, RBlockId(2), vec![])],
                        default: (RBlockId(1), vec![]),
                    },
                },
                RBlock {
                    id: RBlockId(1),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Return(None),
                },
                RBlock {
                    id: RBlockId(2),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Return(None),
                },
            ],
        );
        let r_signed = reachable_blocks(&f_signed, &table);
        assert!(
            r_signed.contains(&RBlockId(1)),
            "-1i32 < 0 branch must remain reachable"
        );

        // Case 3: `-1.0f64 < 0.0f64` must NOT fold IEEE bits unsigned and kill bb1.
        let f_float = make_func(
            vec![t_f64, t_f64, t_bool],
            vec![
                RBlock {
                    id: RBlockId(0),
                    params: vec![],
                    ops: vec![
                        op(
                            Some(0),
                            OpKind::Const(ConstTree::Scalar {
                                bits: u128::from((-1.0f64).to_bits()),
                                bytes: 8,
                            }),
                        ),
                        op(
                            Some(1),
                            OpKind::Const(ConstTree::Scalar {
                                bits: u128::from(0.0f64.to_bits()),
                                bytes: 8,
                            }),
                        ),
                        op(
                            Some(2),
                            OpKind::Binary {
                                op: BinaryOp::Lt,
                                l: ValueId(0),
                                r: ValueId(1),
                                ty: t_f64,
                            },
                        ),
                    ],
                    term: RTerm::Switch {
                        scrut: ValueId(2),
                        scrut_ty: t_bool,
                        cases: vec![(0, RBlockId(2), vec![])],
                        default: (RBlockId(1), vec![]),
                    },
                },
                RBlock {
                    id: RBlockId(1),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Return(None),
                },
                RBlock {
                    id: RBlockId(2),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Return(None),
                },
            ],
        );
        let r_float = reachable_blocks(&f_float, &table);
        assert!(
            r_float.contains(&RBlockId(1)),
            "-1.0f64 < 0.0 branch must remain reachable"
        );
    }

    #[test]
    fn user_enum_named_some_is_not_treated_as_stdlib_wrapper() {
        let mut interns = Interns::default();
        let s_none = interns.intern_str("None");
        let s_some = interns.intern_str("Some");
        let mut table = TypeTable::new();
        let std_crate = 100u64;
        let user_crate = 999u64;
        let std_crates = BTreeSet::from([std_crate]);

        let make_opt = |id: u128, crate_id: u64| {
            make_desc(
                id,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id,
                        local_hash: 42,
                    },
                    args: vec![],
                    form: NominalForm::Enum,
                    variants: vec![
                        VariantDescriptor {
                            name: s_none,
                            fields: vec![],
                            logical_discr: 0,
                            physical_tag: None,
                            result_ok: false,
                            saved_local_keys: vec![],
                        },
                        VariantDescriptor {
                            name: s_some,
                            fields: vec![],
                            logical_discr: 1,
                            physical_tag: None,
                            result_ok: false,
                            saved_local_keys: vec![],
                        },
                    ],
                    niche: None,
                },
                8,
                None,
            )
        };
        table.insert(make_opt(0, std_crate));
        table.insert(make_opt(1, user_crate));

        assert!(is_wrapper_aggregate(
            &table,
            &interns,
            TypeId(0),
            &std_crates
        ));
        assert!(!is_wrapper_aggregate(
            &table,
            &interns,
            TypeId(1),
            &std_crates
        ));
    }

    #[test]
    fn propagate_scalar_origins_intersects_phi_and_cell_paths() {
        let mut table = TypeTable::new();
        let t_u8 = TypeId(0);
        let t_u8_ptr = TypeId(1);
        let t_bool = TypeId(2);
        let t_u32 = TypeId(3);
        let t_f32 = TypeId(4);
        let t_f32_ptr = TypeId(5);
        table.insert(make_desc(
            0,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            1,
            None,
        ));
        table.insert(make_desc(
            1,
            TypeKind::RawPointer {
                pointee: t_u8,
                mutable: false,
            },
            8,
            None,
        ));
        table.insert(make_desc(2, TypeKind::Bool, 1, None));
        table.insert(make_desc(
            3,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            None,
        ));
        table.insert(make_desc(4, TypeKind::Float { bits: 32 }, 4, None));
        table.insert(make_desc(
            5,
            TypeKind::RawPointer {
                pointee: t_f32,
                mutable: false,
            },
            8,
            None,
        ));

        let u32_origin = ScalarOrigin {
            bits: 32,
            signed: false,
            is_char: false,
            size: 4,
        };

        // bb0 switches to bb1 (passing v1 = scalar_scale(u32)) or bb2 (passing v2 = non-scale ptr)
        // both jumping to bb3(v3: *const u8). Must-intersection collapses v3 to unknown (absent).
        let f_phi = make_func(
            vec![t_bool, t_u8_ptr, t_u8_ptr, t_u8_ptr],
            vec![
                RBlock {
                    id: RBlockId(0),
                    params: vec![ValueId(0), ValueId(1), ValueId(2)],
                    ops: vec![],
                    term: RTerm::Switch {
                        scrut: ValueId(0),
                        scrut_ty: t_bool,
                        cases: vec![(0, RBlockId(2), vec![])],
                        default: (RBlockId(1), vec![]),
                    },
                },
                RBlock {
                    id: RBlockId(1),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Goto(RBlockId(3), vec![ValueId(1)]),
                },
                RBlock {
                    id: RBlockId(2),
                    params: vec![],
                    ops: vec![],
                    term: RTerm::Goto(RBlockId(3), vec![ValueId(2)]),
                },
                RBlock {
                    id: RBlockId(3),
                    params: vec![ValueId(3)],
                    ops: vec![],
                    term: RTerm::Return(None),
                },
            ],
        );
        let reached = reachable_blocks(&f_phi, &table);
        let origins = propagate_scalar_origins(
            &f_phi,
            &reached,
            &table,
            &[(ValueId(1), ScalarView::IntOrChar(u32_origin))],
        );
        assert_eq!(origins[ValueId(1).0 as usize], Some(u32_origin));
        assert_eq!(
            origins[ValueId(3).0 as usize],
            None,
            "phi joining u32-scaled ptr with unknown ptr must collapse to Top"
        );

        // Hardening check: casting a u32-scaled pointer to *const f32 must NOT match u32 origin.
        assert!(!matches_unscale_target(
            &table,
            t_f32_ptr,
            Some(4),
            Some(u32_origin)
        ));
        let t_u32_ptr = TypeId(6);
        table.insert(make_desc(
            6,
            TypeKind::RawPointer {
                pointee: t_u32,
                mutable: false,
            },
            8,
            None,
        ));
        assert!(matches_unscale_target(
            &table,
            t_u32_ptr,
            Some(4),
            Some(u32_origin)
        ));
    }

    #[test]
    fn sealed_wide_vec_container_enforces_std_identity_and_element_match() {
        let mut interns = Interns::default();
        let s_vec = interns.intern_str("Vec");
        let mut table = TypeTable::new();
        let std_crate = 100u64;
        let user_crate = 999u64;
        let std_crates = BTreeSet::from([std_crate]);

        let t_u8 = TypeId(0);
        let t_u16 = TypeId(1);
        let t_i16 = TypeId(2);
        let t_u32 = TypeId(3);
        let t_f32 = TypeId(4);
        let t_f64 = TypeId(5);
        let t_rc_like = TypeId(6);
        table.insert(make_desc(
            0,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            1,
            Some((false, 8)),
        ));
        table.insert(make_desc(
            1,
            TypeKind::Integer {
                signed: false,
                bits: 16,
            },
            2,
            Some((false, 16)),
        ));
        table.insert(make_desc(
            2,
            TypeKind::Integer {
                signed: true,
                bits: 16,
            },
            2,
            Some((true, 16)),
        ));
        table.insert(make_desc(
            3,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            4,
            Some((false, 32)),
        ));
        table.insert(make_desc(4, TypeKind::Float { bits: 32 }, 4, None));
        table.insert(make_desc(5, TypeKind::Float { bits: 64 }, 8, None));
        table.insert(make_desc(
            6,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: std_crate,
                    local_hash: 77,
                },
                args: vec![],
                form: NominalForm::Struct,
                variants: vec![],
                niche: None,
            },
            4,
            None,
        ));

        let make_vec = |id: u128, crate_id: u64, elem: TypeId| {
            make_desc(
                id,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id,
                        local_hash: 10,
                    },
                    args: vec![elem],
                    form: NominalForm::Struct,
                    variants: vec![VariantDescriptor {
                        name: s_vec,
                        fields: vec![],
                        logical_discr: 0,
                        physical_tag: None,
                        result_ok: false,
                        saved_local_keys: vec![],
                    }],
                    niche: None,
                },
                24,
                None,
            )
        };
        let v_u8 = TypeId(10);
        let v_u16 = TypeId(11);
        let v_i16 = TypeId(12);
        let v_u32 = TypeId(13);
        let v_f32 = TypeId(14);
        let v_rc = TypeId(15);
        let v_user_u16 = TypeId(16);
        table.insert(make_vec(10, std_crate, t_u8));
        table.insert(make_vec(11, std_crate, t_u16));
        table.insert(make_vec(12, std_crate, t_i16));
        table.insert(make_vec(13, std_crate, t_u32));
        table.insert(make_vec(14, std_crate, t_f32));
        table.insert(make_vec(15, std_crate, t_rc_like));
        table.insert(make_vec(16, user_crate, t_u16));

        let u16_origin = ScalarOrigin {
            bits: 16,
            signed: false,
            is_char: false,
            size: 2,
        };
        // Scalar u16 origin only seals into std Vec<u16>.
        assert!(is_sealed_wide_vec_container(
            &table,
            &interns,
            v_u16,
            &std_crates,
            2,
            t_u16,
            Some(u16_origin)
        ));
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_i16,
            &std_crates,
            2,
            t_u16,
            Some(u16_origin)
        ));
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_u8,
            &std_crates,
            2,
            t_u16,
            Some(u16_origin)
        ));
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_user_u16,
            &std_crates,
            2,
            t_u16,
            Some(u16_origin)
        ));
        // Float f32 only seals into std Vec<f32>, never Vec<u32> or Vec<RcLike>.
        assert!(is_sealed_wide_vec_container(
            &table,
            &interns,
            v_f32,
            &std_crates,
            4,
            t_f32,
            None
        ));
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_u32,
            &std_crates,
            4,
            t_f32,
            None
        ));
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_rc,
            &std_crates,
            4,
            t_f32,
            None
        ));
        // Non-scalar 4-byte Rc-like seals into std Vec<RcLike>, never Vec<u32> or Vec<f32>.
        assert!(is_sealed_wide_vec_container(
            &table,
            &interns,
            v_rc,
            &std_crates,
            4,
            t_rc_like,
            None
        ));
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_u32,
            &std_crates,
            4,
            t_rc_like,
            None
        ));
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_f32,
            &std_crates,
            4,
            t_rc_like,
            None
        ));
        // 8-byte float mismatch rejected.
        assert!(!is_sealed_wide_vec_container(
            &table,
            &interns,
            v_f32,
            &std_crates,
            8,
            t_f64,
            None
        ));
    }
}
