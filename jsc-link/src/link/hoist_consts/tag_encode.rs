//! Internal enum `TAG` dense integer encoding and JIR boundary type walking.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{ItemKind, JsName, LinkItem};
use crate::jir::interns::SigId;
use crate::jir::semantic::{CallSemantic, Callee, Terminator};
use crate::jir::types::{
    FieldDescriptor, FieldKey, NominalAbi, NominalForm, PinnedBoundary, TypeId, TypeKind, TypeTable,
};
use crate::jir::{DefinitionId, Interns, ItemId};
use crate::jsast::{self, BinOp, Expr, Stmt, Visit, VisitMut};

use super::exclusions::ProgramExclusions;
use super::LoweredRow;

struct BoundaryWalkCtx<'a> {
    items_by_id: BTreeMap<ItemId, &'a LinkItem>,
    items_by_def: BTreeMap<DefinitionId, Vec<&'a LinkItem>>,
    visited_types: BTreeSet<(usize, TypeId)>,
    visited_items: BTreeSet<JsName>,
    fields: BTreeSet<String>,
    variants: BTreeSet<String>,
}

impl<'a> BoundaryWalkCtx<'a> {
    fn new(table: &'a BTreeMap<JsName, LinkItem>) -> Self {
        let mut items_by_id = BTreeMap::new();
        let mut items_by_def: BTreeMap<DefinitionId, Vec<&'a LinkItem>> = BTreeMap::new();
        for item in table.values() {
            if let Some(id) = item
                .jir
                .summary
                .as_ref()
                .and_then(|s| s.key.item)
                .or_else(|| item.jir.body.as_ref().map(|b| b.item))
            {
                items_by_id.entry(id).or_insert(item);
                items_by_def.entry(id.definition).or_default().push(item);
            }
        }
        Self {
            items_by_id,
            items_by_def,
            visited_types: BTreeSet::new(),
            visited_items: BTreeSet::new(),
            fields: BTreeSet::new(),
            variants: BTreeSet::new(),
        }
    }

    fn walk_item_signature(&mut self, item: &'a LinkItem) {
        if !self.visited_items.insert(item.name.clone()) {
            return;
        }
        let (Some(interns), Some(body)) = (item.jir.interns(), &item.jir.body) else {
            return;
        };
        for &arg_ty in &body.signature.arguments {
            self.walk_type(arg_ty, interns);
        }
        self.walk_type(body.signature.result, interns);
    }

    fn walk_item_id(&mut self, id: ItemId) {
        if let Some(item) = self.items_by_id.get(&id).copied() {
            self.walk_item_signature(item);
        }
    }

    fn walk_fields(&mut self, fields: &[FieldDescriptor], interns: &'a Interns) {
        for f in fields {
            if let FieldKey::Name(id) = f.key {
                if let Some(name) = interns.str(id) {
                    self.fields.insert(name.to_owned());
                }
            }
            self.walk_type(f.ty, interns);
        }
    }

    fn walk_sig(&mut self, sig: SigId, interns: &'a Interns) {
        if let Some(sig_desc) = interns.sigs.get(sig) {
            for &inp in &sig_desc.inputs {
                self.walk_type(inp, interns);
            }
            self.walk_type(sig_desc.output, interns);
        }
    }

    fn walk_type(&mut self, ty: TypeId, interns: &'a Interns) {
        let types = &interns.types;
        if !self
            .visited_types
            .insert((types as *const TypeTable as usize, ty))
        {
            return;
        }
        let Some(desc) = types.get(ty) else {
            return;
        };
        match &desc.kind {
            TypeKind::Adt {
                form,
                variants: adt_variants,
                args,
                ..
            } => {
                for v in adt_variants {
                    if *form == NominalForm::Enum {
                        if let Some(name) = interns.str(v.name) {
                            self.variants.insert(name.to_owned());
                        }
                    }
                    self.walk_fields(&v.fields, interns);
                }
                for &arg in args {
                    self.walk_type(arg, interns);
                }
            }
            TypeKind::Tuple(elems) => {
                for &el in elems {
                    self.walk_type(el, interns);
                }
            }
            TypeKind::Array { element, .. }
            | TypeKind::Slice(element)
            | TypeKind::Reference {
                pointee: element, ..
            }
            | TypeKind::RawPointer {
                pointee: element, ..
            }
            | TypeKind::Pattern(element) => {
                self.walk_type(*element, interns);
            }
            TypeKind::FnPtr { sig } => self.walk_sig(*sig, interns),
            TypeKind::FnDef { sig, instance, .. } => {
                self.walk_sig(*sig, interns);
                if let Some(direct) = instance {
                    self.walk_item_id(direct.item);
                }
            }
            TypeKind::Closure {
                def,
                upvars,
                fn_pointer_shim,
            } => {
                for &u in upvars {
                    self.walk_type(u, interns);
                }
                if let Some(shim) = fn_pointer_shim {
                    self.walk_item_id(shim.item);
                }
                let exact = ItemId {
                    definition: *def,
                    instantiation: ty.0,
                };
                if let Some(closure_item) = self.items_by_id.get(&exact).copied() {
                    self.walk_item_signature(closure_item);
                } else if let Some(defs) = self.items_by_def.get(def).cloned() {
                    for closure_item in defs {
                        self.walk_item_signature(closure_item);
                    }
                }
            }
            TypeKind::CoroutineClosure { def, upvars } => {
                for &u in upvars {
                    self.walk_type(u, interns);
                }
                if let Some(defs) = self.items_by_def.get(def).cloned() {
                    for closure_item in defs {
                        self.walk_item_signature(closure_item);
                    }
                }
            }
            TypeKind::Coroutine {
                upvars,
                saved_locals,
                variants: co_variants,
                ..
            } => {
                for &u in upvars {
                    self.walk_type(u, interns);
                }
                for &(_, lt) in saved_locals {
                    self.walk_type(lt, interns);
                }
                for v in co_variants {
                    self.walk_fields(&v.fields, interns);
                }
            }
            _ => {}
        }
    }
}

/// Walks JIR boundary types (exports, imports, `.d.ts` items, non-lowered items, host/foreign/raw
/// calls, and pinned/C-ABI types) and returns `(boundary_fields, boundary_variants)`.
pub(super) fn collect_boundary_exclusions(
    table: &BTreeMap<JsName, LinkItem>,
    lowered_ok: &BTreeSet<&JsName>,
    lowered_dts: &BTreeSet<&JsName>,
    live: &BTreeSet<JsName>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut ctx = BoundaryWalkCtx::new(table);

    for item in table.values() {
        if !live.contains(&item.name) {
            continue;
        }
        let Some(interns) = item.jir.interns() else {
            continue;
        };
        for desc in interns.types.descriptors() {
            if desc.meta.boundary != PinnedBoundary::None || desc.meta.abi == NominalAbi::C {
                ctx.walk_type(desc.id, interns);
            }
        }
        let Some(body) = &item.jir.body else {
            continue;
        };
        if item.linkage.exported
            || item.kind == ItemKind::Import
            || item.dts.is_some()
            || lowered_dts.contains(&item.name)
            || !lowered_ok.contains(&item.name)
        {
            ctx.walk_item_signature(item);
        }
        for block in &body.blocks {
            let Terminator::Call(call) = &block.terminator else {
                continue;
            };
            let is_boundary_call = call.host.is_some()
                || matches!(
                    call.semantic,
                    CallSemantic::Host | CallSemantic::RawJavaScript | CallSemantic::RawParts
                )
                || (matches!(call.callee, Callee::Foreign { .. } | Callee::Runtime { .. })
                    && !crate::plan::enum_repr::is_representation_opaque_runtime(
                        &call.callee,
                        interns,
                    ))
                || match &call.callee {
                    Callee::Direct {
                        item: target_id,
                        link_name,
                    } => {
                        let target_name = ctx
                            .items_by_id
                            .get(target_id)
                            .map(|it| it.name.clone())
                            .or_else(|| interns.str(*link_name).map(|s| JsName::new(s.to_owned())));
                        !target_name.as_ref().is_some_and(|n| lowered_ok.contains(n))
                    }
                    _ => false,
                };
            if !is_boundary_call {
                continue;
            }
            for arg in &call.arguments {
                ctx.walk_type(arg.ty, interns);
            }
            ctx.walk_type(call.destination.ty, interns);
            if let Some(host) = &call.host {
                for (_, helper) in host.helpers() {
                    ctx.walk_item_id(helper.item);
                }
            }
        }
    }

    (ctx.fields, ctx.variants)
}

struct TagScanner {
    candidates: BTreeSet<String>,
    bad: BTreeSet<String>,
}

impl Visit for TagScanner {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if let Stmt::Switch(Expr::Member(obj, prop), cases) = stmt {
            if prop == "TAG" {
                self.visit_expr(obj);
                for c in cases {
                    for t in &c.tests {
                        if let Expr::Str(s) = t {
                            self.candidates.insert(s.clone());
                        } else {
                            self.visit_expr(t);
                        }
                    }
                    self.visit_stmts(&c.body);
                }
                return;
            }
        }
        jsast::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Str(s) => {
                self.bad.insert(s.clone());
            }
            Expr::Object(entries) => {
                for (k, val) in entries {
                    if k == "TAG" {
                        if let Expr::Str(s) = val {
                            self.candidates.insert(s.clone());
                            continue;
                        }
                    }
                    self.visit_expr(val);
                }
            }
            Expr::Assign(lhs, rhs) => {
                if let (Expr::Member(obj, k), Expr::Str(s)) = (lhs.as_ref(), rhs.as_ref()) {
                    if k == "TAG" {
                        self.candidates.insert(s.clone());
                        self.visit_expr(obj);
                        return;
                    }
                }
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            Expr::Binary(BinOp::StrictEq | BinOp::StrictNe, l, r) => {
                if let (Expr::Member(obj, k), Expr::Str(s)) = (l.as_ref(), r.as_ref()) {
                    if k == "TAG" {
                        self.candidates.insert(s.clone());
                        self.visit_expr(obj);
                        return;
                    }
                }
                if let (Expr::Str(s), Expr::Member(obj, k)) = (l.as_ref(), r.as_ref()) {
                    if k == "TAG" {
                        self.candidates.insert(s.clone());
                        self.visit_expr(obj);
                        return;
                    }
                }
                self.visit_expr(l);
                self.visit_expr(r);
            }
            _ => jsast::walk_expr(self, expr),
        }
    }
}

struct TagRewriter<'a> {
    tag_map: &'a BTreeMap<String, usize>,
    changed: bool,
}

impl TagRewriter<'_> {
    fn rewrite_str_if_mapped(&mut self, expr: &mut Expr) {
        if let Expr::Str(s) = expr {
            if let Some(&id) = self.tag_map.get(s) {
                *expr = Expr::Num(id as f64);
                self.changed = true;
            }
        }
    }
}

impl VisitMut for TagRewriter<'_> {
    fn visit_stmt_mut(&mut self, stmt: &mut Stmt) {
        if let Stmt::Switch(Expr::Member(obj, prop), cases) = stmt {
            if prop == "TAG" {
                self.visit_expr_mut(obj);
                for c in cases {
                    for t in &mut c.tests {
                        if matches!(t, Expr::Str(_)) {
                            self.rewrite_str_if_mapped(t);
                        } else {
                            self.visit_expr_mut(t);
                        }
                    }
                    self.visit_stmts_mut(&mut c.body);
                }
                return;
            }
        }
        jsast::walk_stmt_mut(self, stmt);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Object(entries) => {
                for (k, val) in entries {
                    if k == "TAG" && matches!(val, Expr::Str(_)) {
                        self.rewrite_str_if_mapped(val);
                    } else {
                        self.visit_expr_mut(val);
                    }
                }
            }
            Expr::Assign(lhs, rhs) => {
                if matches!(&**lhs, Expr::Member(_, k) if k == "TAG")
                    && matches!(&**rhs, Expr::Str(_))
                {
                    self.rewrite_str_if_mapped(rhs);
                    if let Expr::Member(obj, _) = &mut **lhs {
                        self.visit_expr_mut(obj);
                    }
                    return;
                }
                self.visit_expr_mut(lhs);
                self.visit_expr_mut(rhs);
            }
            Expr::Binary(BinOp::StrictEq | BinOp::StrictNe, l, r) => {
                if matches!(&**l, Expr::Member(_, k) if k == "TAG") && matches!(&**r, Expr::Str(_))
                {
                    self.rewrite_str_if_mapped(r);
                    if let Expr::Member(obj, _) = &mut **l {
                        self.visit_expr_mut(obj);
                    }
                    return;
                }
                if matches!(&**r, Expr::Member(_, k) if k == "TAG") && matches!(&**l, Expr::Str(_))
                {
                    self.rewrite_str_if_mapped(l);
                    if let Expr::Member(obj, _) = &mut **r {
                        self.visit_expr_mut(obj);
                    }
                    return;
                }
                self.visit_expr_mut(l);
                self.visit_expr_mut(r);
            }
            _ => jsast::walk_expr_mut(self, expr),
        }
    }
}

pub(super) fn encode_internal_tags(
    lowered: &mut [LoweredRow],
    exclusions: &ProgramExclusions,
    encode_option_tags: bool,
) {
    let mut scanner = TagScanner {
        candidates: BTreeSet::new(),
        bad: BTreeSet::new(),
    };
    for s in ["Ok", "Err", "Ready", "Pending", ""] {
        scanner.bad.insert(s.to_owned());
    }
    if !encode_option_tags {
        scanner.bad.insert("Some".to_owned());
        scanner.bad.insert("None".to_owned());
    }
    scanner.bad.extend(exclusions.raw_tokens.iter().cloned());
    scanner.bad.extend(exclusions.raw_strings.iter().cloned());
    scanner
        .bad
        .extend(exclusions.boundary_variants.iter().cloned());

    for (_, res, _, _) in lowered.iter() {
        if let Ok(item) = res {
            scanner.visit_stmt(&item.decl);
        }
    }

    let tag_map: BTreeMap<String, usize> = scanner
        .candidates
        .into_iter()
        .filter(|s| !scanner.bad.contains(s))
        .enumerate()
        .map(|(idx, s)| (s, idx))
        .collect();
    if tag_map.is_empty() {
        return;
    }

    for (item_name, res, _, _) in lowered.iter_mut() {
        let Ok(item) = res else { continue };
        let mut rewriter = TagRewriter {
            tag_map: &tag_map,
            changed: false,
        };
        rewriter.visit_stmt_mut(&mut item.decl);
        if rewriter.changed {
            item.refs = super::super::lane::refs_of(&item.decl, item_name);
        }
    }
}
