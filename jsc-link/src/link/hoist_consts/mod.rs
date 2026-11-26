//! Post-lowering AST constant hoisting and unreachable-throw outlining.
//!
//! Runs after Phase C lowering when `opts.optimizer == OptimizerMode::On`:
//! - Single-assignment read-only scalar array lookup tables (`len >= 8`, at any statement depth
//!   including inside loops/branches) are hoisted to module-level `const` items.
//! - Repeated or in-loop `Location` literals (`{ filename, line, col }`), ES2018 `BigInt("...")`
//!   calls, and inline scalar array indexing literals (`[...][i]`) are hoisted to module `const`s.
//! - Repeated `throw __rt.abort_error(new Error("rustc_codegen_js: entered unreachable code"))`
//!   statements (`>= 2` occurrences) are outlined into a shared zero-argument helper function.

mod bigint_props;
mod bstr_const;
mod exclusions;
mod tables;
mod tag_encode;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{item_map, ItemKind, JsItem, JsName, LinkItem, Linkage};
use crate::jir::decline::Decline;
use crate::jsast::{self, walk_expr, walk_stmt, ArrowBody, Expr, Stmt, UnOp, Visit};
use crate::lower::LoweredItem;
use crate::opt::OptStats;
use crate::opts::{EcmaTarget, OptimizerMode, Opts};

pub(super) use bstr_const::{rewrite_bstr_location_const_item, synthesize_bstr_const_item};

use self::bigint_props::split_bigint_props;
use self::exclusions::collect_program_exclusions;
use self::tables::{
    collect_declared_locals, exact_int_lit, find_read_only_local_init, is_scalar_array_lit,
    replace_ident_in_stmts, strip_hoisted_table_stmts, upgrade_readonly_static_int_arrays,
    wrap_typed_num_array, ReadOnlyKind,
};
use self::tag_encode::encode_internal_tags;

pub(super) type LoweredRow = (
    JsName,
    Result<LoweredItem, Decline>,
    OptStats,
    Option<String>,
);

fn is_pure_factory_expr(expr: &Expr, allow_bigint: bool) -> bool {
    match expr {
        Expr::Num(_)
        | Expr::RawNum(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Undefined
        | Expr::Null => true,
        Expr::BigInt(_) => allow_bigint,
        Expr::Unary(op, inner) => {
            matches!(op, UnOp::Neg | UnOp::Pos | UnOp::Not | UnOp::BitNot)
                && is_pure_factory_expr(inner, allow_bigint)
        }
        Expr::Call(callee, args) => {
            allow_bigint
                && matches!(&**callee, Expr::Ident(n) if n == "BigInt")
                && args.len() == 1
                && matches!(&args[0], Expr::Str(_) | Expr::Num(_))
        }
        Expr::Array(items) => {
            items.len() <= 32
                && items
                    .iter()
                    .all(|item| is_pure_factory_expr(item, allow_bigint))
        }
        Expr::Object(entries) => {
            entries.len() <= 32
                && entries
                    .iter()
                    .all(|(_, v)| is_pure_factory_expr(v, allow_bigint))
        }
        _ => false,
    }
}

/// Matches a pure inline `new Uint8Array([b0, b1, ...])` lookup table literal (`len >= 8`, all
/// entries exact integers in `0..=255`) such as the 128/256-byte ASCII LUTs synthesized by
/// `post_opt::lower_dense_ascii_luts`.
fn is_u8_lut_lit(expr: &Expr) -> bool {
    let Expr::New(ctor, args) = expr else {
        return false;
    };
    if !matches!(ctor.as_ref(), Expr::Ident(id) if id == "Uint8Array") || args.len() != 1 {
        return false;
    }
    let Expr::Array(elems) = &args[0] else {
        return false;
    };
    elems.len() >= 8
        && elems
            .iter()
            .all(|elem| exact_int_lit(elem).is_some_and(|v| (0..=255).contains(&v)))
}

fn is_location_filename_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Str(_) => true,
        Expr::Call(callee, args) if args.len() == 1 && matches!(&args[0], Expr::Str(_)) => {
            matches!(
                callee.as_ref(),
                Expr::Member(ns, method)
                    if matches!(ns.as_ref(), Expr::Ident(id) if id == "__rt")
                        && matches!(method.as_str(), "str_bytes" | "bstr_bytes")
            )
        }
        _ => false,
    }
}

fn is_location_object(expr: &Expr) -> bool {
    let Expr::Object(entries) = expr else {
        return false;
    };
    if entries.len() != 3 {
        return false;
    }
    matches!(
        (&entries[0], &entries[1], &entries[2]),
        (
            (k0, file_expr),
            (k1, Expr::Num(_) | Expr::RawNum(_)),
            (k2, Expr::Num(_) | Expr::RawNum(_)),
        ) if k0 == "filename" && k1 == "line" && k2 == "col" && is_location_filename_expr(file_expr)
    )
}

fn is_unreachable_abort_expr(expr: &Expr) -> bool {
    let Expr::Call(callee, args) = expr else {
        return false;
    };
    let Expr::Member(ns, method) = callee.as_ref() else {
        return false;
    };
    if !matches!(ns.as_ref(), Expr::Ident(id) if id == "__rt") || method != "abort_error" {
        return false;
    }
    let [Expr::New(err_ctor, err_args)] = args.as_slice() else {
        return false;
    };
    if !matches!(err_ctor.as_ref(), Expr::Ident(id) if id == "Error") {
        return false;
    }
    matches!(
        err_args.as_slice(),
        [Expr::Str(msg)] if msg == "rustc_codegen_js: entered unreachable code"
    )
}

fn is_hoistable_bstr(s: &str) -> bool {
    !s.is_ascii() && s.chars().count() >= crate::lower::js::constant::HOIST_STRING_BYTES
}

#[derive(Default, Clone, Copy)]
struct ConstUsage {
    count: usize,
    in_loop: bool,
}

struct HoistCollector {
    allow_bigint: bool,
    bstr_strings: bool,
    speed_profile: bool,
    allow_u8_lut: bool,
    in_loop: bool,
    counts: BTreeMap<String, (Expr, ConstUsage)>,
    unreachable_throws: usize,
}

impl HoistCollector {
    fn record(&mut self, key: String, expr: &Expr) {
        let entry = self
            .counts
            .entry(key)
            .or_insert_with(|| (expr.clone(), ConstUsage::default()));
        entry.1.count += 1;
        entry.1.in_loop |= self.in_loop;
    }

    fn scan_expr(&mut self, expr: &Expr, is_assign_target: bool) {
        if !is_assign_target {
            if is_location_object(expr) {
                self.record(format!("loc:{expr}"), expr);
                return;
            }
            if let Expr::Index(obj, idx) = expr {
                if self.allow_u8_lut && is_u8_lut_lit(obj) {
                    self.record(format!("u8lut:{obj}"), obj);
                    self.scan_expr(idx, false);
                    return;
                }
                let min_inline_arr = if self.speed_profile { 2 } else { 4 };
                if is_scalar_array_lit(obj, min_inline_arr, self.allow_bigint) {
                    self.record(format!("arr:{obj}"), obj);
                    self.scan_expr(idx, false);
                    return;
                }
            }
            if let Expr::Arrow(params, ArrowBody::Expr(inner)) = expr {
                if params.is_empty() && is_pure_factory_expr(inner, self.allow_bigint) {
                    self.record(format!("fn:{expr}"), expr);
                    return;
                }
            }
            if self.bstr_strings {
                if let Expr::Str(s) = expr {
                    if is_hoistable_bstr(s) {
                        self.record(format!("str:{s}"), expr);
                        return;
                    }
                }
            }
        }
        if let Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) = expr {
            self.scan_expr(l, true);
            self.scan_expr(r, false);
            return;
        }
        walk_expr(self, expr);
    }
}

impl Visit for HoistCollector {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Throw(e) => {
                if is_unreachable_abort_expr(e) {
                    self.unreachable_throws += 1;
                } else {
                    self.scan_expr(e, false);
                }
            }
            Stmt::LoopForever(_, body) => {
                let prev = self.in_loop;
                self.in_loop = true;
                self.visit_stmts(body);
                self.in_loop = prev;
            }
            Stmt::While(test, body) => {
                let prev = self.in_loop;
                self.in_loop = true;
                self.scan_expr(test, false);
                self.visit_stmts(body);
                self.in_loop = prev;
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        self.scan_expr(expr, false);
    }
}

struct HoistRewriter<'a> {
    allow_bigint: bool,
    bstr_strings: bool,
    speed_profile: bool,
    allow_u8_lut: bool,
    const_map: &'a BTreeMap<String, JsName>,
    unreachable_fn: Option<&'a JsName>,
    changed: bool,
}

impl HoistRewriter<'_> {
    fn rewrite_stmt(&mut self, stmt: &mut Stmt) {
        jsast::VisitMut::visit_stmt_mut(self, stmt);
    }

    fn rewrite_expr(&mut self, expr: &mut Expr, is_assign_target: bool) {
        if !is_assign_target {
            if is_location_object(expr) {
                let key = format!("loc:{expr}");
                if let Some(hoisted) = self.const_map.get(&key) {
                    *expr = Expr::Ident(hoisted.as_str().to_owned());
                    self.changed = true;
                    return;
                }
            }
            if let Expr::Index(obj, idx) = expr {
                if self.allow_u8_lut && is_u8_lut_lit(obj) {
                    let key = format!("u8lut:{obj}");
                    if let Some(hoisted) = self.const_map.get(&key) {
                        **obj = Expr::Ident(hoisted.as_str().to_owned());
                        self.changed = true;
                        self.rewrite_expr(idx, false);
                        return;
                    }
                }
                let min_inline_arr = if self.speed_profile { 2 } else { 4 };
                if is_scalar_array_lit(obj, min_inline_arr, self.allow_bigint) {
                    let key = format!("arr:{obj}");
                    if let Some(hoisted) = self.const_map.get(&key) {
                        **obj = Expr::Ident(hoisted.as_str().to_owned());
                        self.changed = true;
                        self.rewrite_expr(idx, false);
                        return;
                    }
                }
            }
            if let Expr::Arrow(params, ArrowBody::Expr(inner)) = expr {
                if params.is_empty() && is_pure_factory_expr(inner, self.allow_bigint) {
                    let key = format!("fn:{expr}");
                    if let Some(hoisted) = self.const_map.get(&key) {
                        *expr = Expr::Ident(hoisted.as_str().to_owned());
                        self.changed = true;
                        return;
                    }
                }
            }
            if self.bstr_strings {
                if let Expr::Str(s) = expr {
                    if is_hoistable_bstr(s) {
                        if let Some(hoisted) = self.const_map.get(&format!("str:{s}")) {
                            *expr = Expr::Ident(hoisted.as_str().to_owned());
                            self.changed = true;
                            return;
                        }
                    }
                }
            }
        }
        if let Expr::Assign(l, r) | Expr::CompoundAssign(_, l, r) = expr {
            self.rewrite_expr(l, true);
            self.rewrite_expr(r, false);
            return;
        }
        jsast::walk_expr_mut(self, expr);
    }
}

impl jsast::VisitMut for HoistRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if let Stmt::Throw(e) = stmt {
            if let Some(ur_name) = self.unreachable_fn {
                if is_unreachable_abort_expr(e) {
                    *e = Expr::Call(
                        Box::new(Expr::Ident(ur_name.as_str().to_owned())),
                        Vec::new(),
                    );
                    self.changed = true;
                    return;
                }
            }
        }
        jsast::walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        self.rewrite_expr(expr, false);
    }
}

fn fresh_item_name(
    prefix: &str,
    counter: &mut usize,
    table: &BTreeMap<JsName, LinkItem>,
    defined: &BTreeSet<JsName>,
) -> JsName {
    loop {
        let candidate = JsName::new(format!("{prefix}_{}", *counter));
        *counter += 1;
        if !table.contains_key(&candidate) && !defined.contains(&candidate) {
            return candidate;
        }
    }
}

pub(super) fn build_synthesized_link_item(
    name: JsName,
    kind: ItemKind,
    decl: Stmt,
    debug_path: &str,
) -> LinkItem {
    let refs = super::lane::refs_of(&decl, &name);
    let mut js_item = JsItem {
        name: name.clone(),
        kind,
        decl,
        refs: refs.clone(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: debug_path.to_owned(),
        order: None,
        loc: None,
        names: Vec::new(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    };
    crate::minify::shorten_locals(std::slice::from_mut(&mut js_item));
    let (js, mappings) = js_item.to_js_mapped(crate::opts::get().comments);
    LinkItem {
        name,
        kind,
        js,
        refs,
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: debug_path.to_owned(),
        crate_name: "generated".to_owned(),
        order: None,
        map: item_map(mappings),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    }
}

fn decl_shadows_ident(stmt: &Stmt, target: &str) -> bool {
    struct V<'a> {
        target: &'a str,
        found: bool,
    }
    impl Visit for V<'_> {
        fn visit_stmt(&mut self, s: &Stmt) {
            if self.found {
                return;
            }
            let hit = match s {
                Stmt::Let(n, _) | Stmt::Const(n, _) => n == self.target,
                Stmt::Lets(decls) => decls.iter().any(|(n, _)| n == self.target),
                Stmt::TryCatch { binding, .. } => binding == self.target,
                Stmt::FunctionDecl { name, params, .. } => {
                    name == self.target || params.iter().any(|p| p == self.target)
                }
                _ => false,
            };
            if hit {
                self.found = true;
            } else {
                walk_stmt(self, s);
            }
        }
        fn visit_expr(&mut self, _: &Expr) {}
    }
    let mut v = V {
        target,
        found: false,
    };
    v.visit_stmt(stmt);
    v.found
}

/// Hoists read-only scalar lookup tables, `BigInt("...")` constants, repeated/loop `Location`
/// objects, and outlines repeated `entered unreachable code` throws into a shared helper.
///
/// Only runs when `opts.optimizer == OptimizerMode::On`. Returns `true` if any synthesized module
/// item was inserted into `table` (signaling downstream reachability recomputation).
pub(crate) fn run(
    table: &mut BTreeMap<JsName, LinkItem>,
    lowered: &mut [LoweredRow],
    defined: &mut BTreeSet<JsName>,
    live: &mut BTreeSet<JsName>,
) -> bool {
    run_with_opts(table, lowered, defined, live, crate::opts::get())
}

fn run_with_opts(
    table: &mut BTreeMap<JsName, LinkItem>,
    lowered: &mut [LoweredRow],
    defined: &mut BTreeSet<JsName>,
    live: &mut BTreeSet<JsName>,
    opts: &Opts,
) -> bool {
    if opts.optimizer != OptimizerMode::On {
        return false;
    }

    let mut added_any = false;
    let mut counter = 0usize;
    let mut synthesized: Vec<LinkItem> = Vec::new();
    let mut table_dedup: BTreeMap<String, JsName> = BTreeMap::new();
    let allow_bigint = opts.ecma_target != EcmaTarget::Es2018;
    let speed_profile = opts.profile == Some(crate::opts::Profile::ReleaseSpeed);

    // Pass 1: Hoist single-assignment read-only scalar lookup tables at any nesting depth.
    // Under `ReleaseSpeed`, also hoist small constant numeric/scalar array literals (`len >= 2`,
    // e.g. 4-word ASCII bitmasks inside loops) into module-scope TypedArrays.
    let table_min_len = if speed_profile { 2 } else { 8 };
    for (item_name, res, _, _) in lowered.iter_mut() {
        let Ok(lowered_item) = res else { continue };
        let Stmt::FunctionDecl { params, body, .. } = &mut lowered_item.decl else {
            continue;
        };
        let param_set: BTreeSet<String> = params.iter().cloned().collect();
        let mut declared_locals: Vec<String> = Vec::new();
        collect_declared_locals(body, &param_set, &mut declared_locals);

        let mut candidates: Vec<(String, Expr, bool)> = Vec::new();
        for local in &declared_locals {
            if let Some(arr_expr) = find_read_only_local_init(
                body,
                local,
                allow_bigint,
                ReadOnlyKind::ScalarTable {
                    min_len: table_min_len,
                },
            ) {
                candidates.push((local.clone(), arr_expr, true));
            } else if let Some(nested_expr) =
                find_read_only_local_init(body, local, allow_bigint, ReadOnlyKind::NestedArray)
            {
                candidates.push((local.clone(), nested_expr, false));
            } else if let Some(slice_expr) =
                find_read_only_local_init(body, local, allow_bigint, ReadOnlyKind::ConstSlice)
            {
                candidates.push((local.clone(), slice_expr, false));
            }
        }

        if !candidates.is_empty() {
            let mut hoisted_set: BTreeSet<String> = BTreeSet::new();
            let mut replacements: Vec<(String, JsName)> = Vec::new();
            for (local, arr_expr, is_read_only_table) in candidates {
                let arr_expr = if speed_profile && is_read_only_table {
                    wrap_typed_num_array(arr_expr, table_min_len)
                } else {
                    arr_expr
                };
                let key = format!("arr:{arr_expr}");
                let hc_name = if let Some(existing) = table_dedup.get(&key) {
                    existing.clone()
                } else {
                    let new_name = fresh_item_name("$hc", &mut counter, table, defined);
                    defined.insert(new_name.clone());
                    live.insert(new_name.clone());
                    let item = build_synthesized_link_item(
                        new_name.clone(),
                        ItemKind::Const,
                        jsast::const_(new_name.as_str(), arr_expr),
                        "hoisted constant table",
                    );
                    synthesized.push(item);
                    table_dedup.insert(key, new_name.clone());
                    added_any = true;
                    new_name
                };
                hoisted_set.insert(local.clone());
                replacements.push((local, hc_name));
            }

            strip_hoisted_table_stmts(body, &hoisted_set, table_min_len);
            for (local, hc_name) in replacements {
                replace_ident_in_stmts(body, &local, hc_name.as_str());
            }
            lowered_item.refs = super::lane::refs_of(&lowered_item.decl, item_name);
        }
    }

    // Whole-program `BigInt` object property `(lo, hi)` splitting and internal enum `TAG` integer
    // encoding run before Pass 2 so eliminated `BigInt` constants are never hoisted as ES2018
    // `BigInt("...")` bindings.
    let exclusions = collect_program_exclusions(table, lowered, live);
    split_bigint_props(lowered, &exclusions);
    encode_internal_tags(lowered, &exclusions, speed_profile);

    // Pass 2: Expression constants (`Location` objects, inline `[...][i]`, inline `new Uint8Array([...])[i]`
    // LUTs, capture-free `() => <const>`) and shared unreachable-abort outlining. ES2018 BigInt
    // constants are hoisted afterwards by `lower::bigint::hoist` with load-safe `$BI_ok` / `$BI_no()` guards.
    let bstr_strings = crate::lower::bstr::enabled();
    let mut collector = HoistCollector {
        allow_bigint,
        bstr_strings,
        speed_profile,
        allow_u8_lut: true,
        in_loop: false,
        counts: BTreeMap::new(),
        unreachable_throws: 0,
    };
    for (_, res, _, _) in lowered.iter() {
        if let Ok(lowered_item) = res {
            collector.allow_u8_lut = !decl_shadows_ident(&lowered_item.decl, "Uint8Array");
            collector.visit_stmt(&lowered_item.decl);
        }
    }

    let mut const_map: BTreeMap<String, JsName> = BTreeMap::new();
    for (key, (expr, usage)) in &collector.counts {
        let qualify = if key.starts_with("u8lut:") {
            true
        } else if key.starts_with("loc:") {
            usage.in_loop || usage.count >= 2 || speed_profile
        } else if key.starts_with("str:") {
            usage.in_loop || usage.count >= 2
        } else if key.starts_with("arr:") {
            let len = match expr {
                Expr::Array(elems) => elems.len(),
                _ => 0,
            };
            usage.in_loop || usage.count >= 2 || len >= 8 || (speed_profile && len >= 4)
        } else {
            key.starts_with("fn:")
        };
        if !qualify {
            continue;
        }
        let init_expr = if speed_profile && key.starts_with("arr:") {
            wrap_typed_num_array(expr.clone(), 2)
        } else {
            expr.clone()
        };
        let dedup_key = if (speed_profile && key.starts_with("arr:")) || key.starts_with("u8lut:") {
            format!("arr:{init_expr}")
        } else {
            key.clone()
        };
        let hc_name = if let Some(existing) = table_dedup.get(&dedup_key) {
            existing.clone()
        } else {
            let new_name = fresh_item_name("$hc", &mut counter, table, defined);
            defined.insert(new_name.clone());
            live.insert(new_name.clone());
            let mut decl = jsast::const_(new_name.as_str(), init_expr);
            if bstr_strings {
                crate::lower::bstr::rewrite(&mut decl);
            }
            let item = build_synthesized_link_item(
                new_name.clone(),
                ItemKind::Const,
                decl,
                "hoisted constant",
            );
            synthesized.push(item);
            table_dedup.insert(dedup_key, new_name.clone());
            added_any = true;
            new_name
        };
        const_map.insert(key.clone(), hc_name);
    }

    let unreachable_fn = if collector.unreachable_throws >= 2 {
        let ur_name = fresh_item_name("$ur", &mut counter, table, defined);
        defined.insert(ur_name.clone());
        live.insert(ur_name.clone());
        let abort_expr = Expr::Call(
            Box::new(Expr::Member(
                Box::new(Expr::Ident("__rt".to_owned())),
                "abort_error".to_owned(),
            )),
            vec![Expr::New(
                Box::new(Expr::Ident("Error".to_owned())),
                vec![Expr::Str(
                    "rustc_codegen_js: entered unreachable code".to_owned(),
                )],
            )],
        );
        let decl = jsast::function(
            ur_name.as_str(),
            Vec::new(),
            vec![Stmt::Return(Some(abort_expr))],
        );
        let item = build_synthesized_link_item(
            ur_name.clone(),
            ItemKind::Fn,
            decl,
            "shared unreachable error helper",
        );
        synthesized.push(item);
        added_any = true;
        Some(ur_name)
    } else {
        None
    };

    if !const_map.is_empty() || unreachable_fn.is_some() {
        for (item_name, res, _, _) in lowered.iter_mut() {
            let Ok(lowered_item) = res else { continue };
            let allow_u8_lut = !decl_shadows_ident(&lowered_item.decl, "Uint8Array");
            let mut rewriter = HoistRewriter {
                allow_bigint,
                bstr_strings,
                speed_profile,
                allow_u8_lut,
                const_map: &const_map,
                unreachable_fn: unreachable_fn.as_ref(),
                changed: false,
            };
            rewriter.rewrite_stmt(&mut lowered_item.decl);
            if rewriter.changed {
                lowered_item.refs = super::lane::refs_of(&lowered_item.decl, item_name);
            }
        }
    }

    // Pass 3: Upgrade internal read-only top-level integer static array items (`CRCT`, `DEO`, etc.)
    // to `const NAME = new <TypedArray>([...])` under `ReleaseSpeed`.
    if speed_profile {
        upgrade_readonly_static_int_arrays(table, lowered, live, &exclusions);
    }

    for item in synthesized {
        table.insert(item.name.clone(), item);
    }

    added_any
}
