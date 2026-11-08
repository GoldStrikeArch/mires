//! Fresh JavaScript allocation sites in one final lowered declaration.
//!
//! This analysis runs after carriers, adapters, optimization, structuring, and JavaScript
//! lowering. It counts the syntax that the emitted program executes to create fresh values:
//! object literals, array literals, arrow closures, and `new`. Unlike JIR allocation accounting,
//! it sees record-shaped pointer views and multi-result arrays introduced by lowering itself.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::jir::repr::{Func, OpKind, RTerm, ValueId};
use crate::jir::semantic::CastKind;
use crate::jir::types::{PointerForm, TypeKind};
use crate::jir::Interns;
use crate::jsast::{ArrowBody, Expr, Stmt};
use crate::lane_report::push_string;
use crate::lower::plan::Plan;

/// The JIR value and boundary behind one JavaScript allocation expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    expression: Expr,
    value: String,
    jir_site: String,
    rust_type: String,
    physical_shape: String,
    reason: String,
}

impl Origin {
    /// Builds provenance for one expression produced while lowering `value`.
    pub fn of(
        expression: Expr,
        value: ValueId,
        func: &Func,
        interns: &Interns,
        plan: &Plan,
    ) -> Origin {
        let ty = func.ty(value).unwrap_or(crate::jir::TypeId(0));
        Origin {
            expression,
            value: format!("v{}", value.0),
            jir_site: definition_site(func, value),
            rust_type: super::alloc_sites::type_name(interns, ty),
            physical_shape: physical_shape(func, interns, plan, value),
            reason: materialization_reason(func, value),
        }
    }
}

/// One kind of fresh JavaScript value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Object,
    Array,
    Closure,
    New,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::Closure => "closure",
            Self::New => "new",
        }
    }
}

/// One fresh-value expression in a structured JavaScript loop.
#[derive(Clone, Debug, PartialEq)]
struct Site {
    index: u32,
    kind: Kind,
    depth: u32,
    origin: Option<Origin>,
}

/// Final JavaScript allocation accounting for one item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JsAllocSites {
    object: u32,
    array: u32,
    closure: u32,
    new: u32,
    in_loop: u32,
    /// Raw JavaScript is structurally opaque. It is reported so a gate cannot mistake an unknown
    /// body for a proven allocation-free one.
    opaque: u32,
    opaque_in_loop: u32,
    direct_calls: BTreeSet<String>,
    loop_calls: BTreeSet<String>,
    sites: Vec<Site>,
}

impl JsAllocSites {
    /// Counts fresh-value expressions in a final lowered declaration.
    pub fn of(stmt: &Stmt) -> JsAllocSites {
        let mut sites = JsAllocSites::default();
        Walker {
            sites: &mut sites,
            depth: 0,
            origins: &[],
        }
        .stmt(stmt);
        sites
    }

    /// Counts sites and attaches the JIR expression provenance captured during lowering.
    pub fn of_with_origins(stmt: &Stmt, origins: &[Origin]) -> JsAllocSites {
        let mut sites = JsAllocSites::default();
        Walker {
            sites: &mut sites,
            depth: 0,
            origins,
        }
        .stmt(stmt);
        sites
    }

    /// The schema-11 sidecar block.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\"object\":{},\"array\":{},\"closure\":{},\"new\":{},\
             \"in_loop\":{},\"opaque\":{},\"opaque_in_loop\":{},\"sites\":[",
            self.object,
            self.array,
            self.closure,
            self.new,
            self.in_loop,
            self.opaque,
            self.opaque_in_loop,
        );
        for (position, site) in self.sites.iter().enumerate() {
            if position > 0 {
                out.push(',');
            }
            let _ = write!(out, "{{\"site\":\"e{}\",\"kind\":", site.index);
            push_string(&mut out, site.kind.as_str());
            let _ = write!(out, ",\"depth\":{}", site.depth);
            if let Some(origin) = &site.origin {
                out.push_str(",\"value\":");
                push_string(&mut out, &origin.value);
                out.push_str(",\"jir_site\":");
                push_string(&mut out, &origin.jir_site);
                out.push_str(",\"rust_type\":");
                push_string(&mut out, &origin.rust_type);
                out.push_str(",\"physical_shape\":");
                push_string(&mut out, &origin.physical_shape);
                out.push_str(",\"materialization_reason\":");
                push_string(&mut out, &origin.reason);
            } else {
                out.push_str(
                    ",\"value\":null,\"jir_site\":null,\"rust_type\":\"unknown\",\
                     \"physical_shape\":\"unknown\",\
                     \"materialization_reason\":\"lowering-generated\"",
                );
            }
            out.push('}');
        }
        out.push_str("]}");
        out
    }

    /// The allocation penalty in minified-byte equivalents. Straight-line sites pay `weight`
    /// once; sites inside structured loops pay `weight * 4^min(depth, 3)`.
    pub fn weighted_term(&self, weight: u64) -> u64 {
        let total = u64::from(self.object)
            + u64::from(self.array)
            + u64::from(self.closure)
            + u64::from(self.new);
        let straight_line = total.saturating_sub(u64::from(self.in_loop));
        let looped = self
            .sites
            .iter()
            .map(|site| 4u64.pow(site.depth.min(3)))
            .sum::<u64>();
        weight.saturating_mul(straight_line.saturating_add(looped))
    }

    /// Total statically present fresh-value sites in this declaration.
    pub fn total(&self) -> u64 {
        u64::from(self.object)
            + u64::from(self.array)
            + u64::from(self.closure)
            + u64::from(self.new)
    }

    /// Fresh-value sites nested in structured JavaScript loops.
    pub fn in_loop(&self) -> u64 {
        u64::from(self.in_loop)
    }

    /// Direct item calls in this declaration. Runtime member calls are present too, but cannot
    /// match an item in the link table and are ignored by the physical-cost call graph.
    pub fn direct_calls(&self) -> &BTreeSet<String> {
        &self.direct_calls
    }

    /// Direct item calls lexically nested in a structured JavaScript loop.
    pub fn loop_calls(&self) -> &BTreeSet<String> {
        &self.loop_calls
    }

    fn record(&mut self, kind: Kind, depth: u32, origin: Option<Origin>) {
        match kind {
            Kind::Object => self.object += 1,
            Kind::Array => self.array += 1,
            Kind::Closure => self.closure += 1,
            Kind::New => self.new += 1,
        }
        if depth > 0 {
            self.in_loop += 1;
            self.sites.push(Site {
                index: self.object + self.array + self.closure + self.new,
                kind,
                depth,
                origin,
            });
        }
    }

    fn raw(&mut self, depth: u32) {
        self.opaque += 1;
        if depth > 0 {
            self.opaque_in_loop += 1;
        }
    }
}

struct Walker<'a> {
    sites: &'a mut JsAllocSites,
    depth: u32,
    origins: &'a [Origin],
}

impl Walker<'_> {
    fn record(&mut self, kind: Kind, expression: &Expr) {
        let origin = self
            .origins
            .iter()
            .find(|origin| origin.expression == *expression)
            .cloned();
        self.sites.record(kind, self.depth, origin);
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ExprStmt(expr) | Stmt::Throw(expr) => self.expr(expr),
            Stmt::Let(_, init) => {
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            Stmt::Lets(declarations) => {
                for (_, init) in declarations {
                    if let Some(init) = init {
                        self.expr(init);
                    }
                }
            }
            Stmt::Const(_, init) => self.expr(init),
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            Stmt::If(test, then, els) => {
                self.expr(test);
                self.stmts(then);
                if let Some(els) = els {
                    self.stmts(els);
                }
            }
            Stmt::Switch(discriminant, cases) => {
                self.expr(discriminant);
                for case in cases {
                    for test in &case.tests {
                        self.expr(test);
                    }
                    self.stmts(&case.body);
                }
            }
            Stmt::Labeled(_, inner) => self.stmt(inner),
            Stmt::LoopForever(_, body) => self.in_loop(None, body),
            Stmt::While(test, body) => self.in_loop(Some(test), body),
            Stmt::FunctionDecl { body, .. } => {
                let depth = self.depth;
                self.depth = 0;
                self.stmts(body);
                self.depth = depth;
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.stmts(body);
                self.stmts(handler);
            }
            Stmt::Block(body) => self.stmts(body),
            Stmt::Raw(_) => self.sites.raw(self.depth),
            Stmt::Import { .. }
            | Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Comment(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn in_loop(&mut self, test: Option<&Expr>, body: &[Stmt]) {
        self.depth += 1;
        if let Some(test) = test {
            self.expr(test);
        }
        self.stmts(body);
        self.depth -= 1;
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Member(object, _) => self.expr(object),
            Expr::Index(object, index) => {
                self.expr(object);
                self.expr(index);
            }
            Expr::Call(callee, arguments) => {
                if let Expr::Ident(name) = callee.as_ref() {
                    self.sites.direct_calls.insert(name.clone());
                    if self.depth > 0 {
                        self.sites.loop_calls.insert(name.clone());
                    }
                }
                self.expr(callee);
                for argument in arguments {
                    self.expr(argument);
                }
            }
            Expr::New(callee, arguments) => {
                self.record(Kind::New, expr);
                self.expr(callee);
                for argument in arguments {
                    self.expr(argument);
                }
            }
            Expr::Unary(_, operand) => self.expr(operand),
            Expr::Binary(_, left, right)
            | Expr::Assign(left, right)
            | Expr::CompoundAssign(_, left, right) => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test);
                self.expr(consequent);
                self.expr(alternate);
            }
            Expr::Object(entries) => {
                self.record(Kind::Object, expr);
                for (_, value) in entries {
                    self.expr(value);
                }
            }
            Expr::Array(items) => {
                self.record(Kind::Array, expr);
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Arrow(_, body) => {
                self.record(Kind::Closure, expr);
                let depth = self.depth;
                self.depth = 0;
                match body {
                    ArrowBody::Expr(body) => self.expr(body),
                    ArrowBody::Block(body) => self.stmts(body),
                }
                self.depth = depth;
            }
            Expr::Seq(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Raw(_) => self.sites.raw(self.depth),
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::Ident(_) => {}
        }
    }
}

/// The defining JIR position of a value.
fn definition_site(func: &Func, value: ValueId) -> String {
    for block in &func.blocks {
        if let Some(position) = block.params.iter().position(|param| *param == value) {
            return format!("b{}:param{position}", block.id.0);
        }
        if let Some(position) = block.ops.iter().position(|op| op.result == Some(value)) {
            return format!("b{}:{position}", block.id.0);
        }
    }
    "entry-param".to_owned()
}

/// The JavaScript carrier that lowering selected for a value.
fn physical_shape(func: &Func, interns: &Interns, plan: &Plan, value: ValueId) -> String {
    if plan.physical_slices.contains(&value) {
        return "scalar-lanes(buf,off,len)".to_owned();
    }
    if plan.physical_slots.contains(&value) {
        return "scalar-lanes(buf,off)".to_owned();
    }
    let Some(ty) = func.ty(value) else {
        return "unknown".to_owned();
    };
    match interns.types.pointer_form(ty) {
        Some(PointerForm::Slot) => "record(buf,off,sc?)".to_owned(),
        Some(PointerForm::FatSlice) => "record(buf,off,len)".to_owned(),
        Some(PointerForm::FatDyn | PointerForm::FatTail) => "record(ptr,meta)".to_owned(),
        Some(PointerForm::Thin) => "scalar-reference".to_owned(),
        Some(PointerForm::Str) => "host-string".to_owned(),
        None => match interns
            .types
            .get(interns.types.peeled(ty))
            .map(|descriptor| &descriptor.kind)
        {
            Some(TypeKind::Tuple(_) | TypeKind::Array { .. }) => "array-product".to_owned(),
            Some(TypeKind::Adt { form, .. }) => format!("{form:?}-object").to_lowercase(),
            Some(TypeKind::Closure { .. }) => "closure-environment".to_owned(),
            _ => "scalar-or-opaque".to_owned(),
        },
    }
}

/// The first boundary that forces a freshly lowered value to stay materialized.
fn materialization_reason(func: &Func, value: ValueId) -> String {
    let mut work = vec![value];
    let mut seen = BTreeSet::new();
    while let Some(value) = work.pop() {
        if !seen.insert(value) {
            continue;
        }
        for block in &func.blocks {
            for op in &block.ops {
                if !op.kind.uses().contains(&value) {
                    continue;
                }
                match &op.kind {
                    OpKind::Copy(source) | OpKind::Move(source) if *source == value => {
                        if let Some(result) = op.result {
                            work.push(result);
                        }
                    }
                    OpKind::Cast {
                        kind:
                            CastKind::Transmute
                            | CastKind::PtrToPtr
                            | CastKind::PointerCoercion
                            | CastKind::Subtype,
                        v,
                        ..
                    } if *v == value => {
                        if let Some(result) = op.result {
                            work.push(result);
                        }
                    }
                    OpKind::StoreThrough { v, .. } | OpKind::SetField { v, .. } if *v == value => {
                        return "stored-through-addressable-field".to_owned();
                    }
                    OpKind::Call { args, host, .. } if args.contains(&value) => {
                        return if host.is_some() {
                            "host-boundary"
                        } else {
                            "direct-call-boundary"
                        }
                        .to_owned();
                    }
                    OpKind::Host { .. } => return "host-boundary".to_owned(),
                    OpKind::Aggregate { fields, .. } if fields.contains(&value) => {
                        return "nested-product-field".to_owned();
                    }
                    OpKind::Materialize(v) if *v == value => {
                        return "explicit-materialization".to_owned();
                    }
                    _ => {}
                }
            }
            match &block.term {
                RTerm::Return(Some(returned)) if *returned == value => {
                    return "return-boundary".to_owned();
                }
                RTerm::Throw(Some(thrown)) if *thrown == value => {
                    return "throw-boundary".to_owned();
                }
                _ if block
                    .term
                    .edges()
                    .into_iter()
                    .any(|(_, args)| args.contains(&value)) =>
                {
                    return "block-argument-boundary".to_owned();
                }
                _ => {}
            }
        }
    }
    "record-valued-operation".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_literals_are_counted_at_their_structured_loop_depth() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: Vec::new(),
            body: vec![
                Stmt::Let("a".to_owned(), Some(Expr::Array(Vec::new()))),
                Stmt::While(
                    Expr::Bool(true),
                    vec![Stmt::ExprStmt(Expr::Object(Vec::new()))],
                ),
            ],
        };
        let sites = JsAllocSites::of(&decl);
        assert_eq!(sites.array, 1);
        assert_eq!(sites.object, 1);
        assert_eq!(sites.in_loop, 1);
        assert_eq!(sites.sites[0].kind, Kind::Object);
        assert_eq!(sites.weighted_term(24), 24 * 5);
    }

    #[test]
    fn direct_calls_retain_structured_loop_hotness() {
        let decl = Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: Vec::new(),
            body: vec![
                Stmt::ExprStmt(crate::jsast::call(
                    Expr::Ident("cold".to_owned()),
                    Vec::new(),
                )),
                Stmt::While(
                    Expr::Bool(true),
                    vec![Stmt::ExprStmt(crate::jsast::call(
                        Expr::Ident("hot".to_owned()),
                        Vec::new(),
                    ))],
                ),
            ],
        };
        let sites = JsAllocSites::of(&decl);
        assert_eq!(
            sites.direct_calls(),
            &["cold".to_owned(), "hot".to_owned()].into_iter().collect()
        );
        assert_eq!(
            sites.loop_calls(),
            &["hot".to_owned()].into_iter().collect()
        );
    }

    #[test]
    fn an_arrow_is_created_at_the_outer_depth_but_its_body_has_its_own_depth() {
        let decl = Stmt::While(
            Expr::Bool(true),
            vec![Stmt::ExprStmt(Expr::Arrow(
                Vec::new(),
                ArrowBody::Expr(Box::new(Expr::Array(Vec::new()))),
            ))],
        );
        let sites = JsAllocSites::of(&decl);
        assert_eq!(sites.closure, 1);
        assert_eq!(sites.array, 1);
        assert_eq!(sites.in_loop, 1);
        assert_eq!(sites.sites[0].kind, Kind::Closure);
    }

    #[test]
    fn schema_eight_sites_publish_the_forcing_jir_boundary() {
        let allocation = Expr::Object(Vec::new());
        let decl = Stmt::While(Expr::Bool(true), vec![Stmt::ExprStmt(allocation.clone())]);
        let sites = JsAllocSites::of_with_origins(
            &decl,
            &[Origin {
                expression: allocation,
                value: "v268".to_owned(),
                jir_site: "b68:1".to_owned(),
                rust_type: "*mut u8".to_owned(),
                physical_shape: "record(buf,off,sc?)".to_owned(),
                reason: "stored-through-addressable-field".to_owned(),
            }],
        );
        assert_eq!(
            sites.to_json(),
            "{\"object\":1,\"array\":0,\"closure\":0,\"new\":0,\"in_loop\":1,\
             \"opaque\":0,\"opaque_in_loop\":0,\"sites\":[{\"site\":\"e1\",\
             \"kind\":\"object\",\"depth\":1,\"value\":\"v268\",\"jir_site\":\"b68:1\",\
             \"rust_type\":\"*mut u8\",\"physical_shape\":\"record(buf,off,sc?)\",\
             \"materialization_reason\":\"stored-through-addressable-field\"}]}"
        );
    }
    #[test]
    fn try_and_catch_allocations_keep_the_surrounding_loop_depth() {
        let stmt = Stmt::While(
            Expr::Bool(true),
            vec![Stmt::TryCatch {
                body: vec![Stmt::ExprStmt(Expr::Array(Vec::new()))],
                binding: "error".to_owned(),
                handler: vec![Stmt::ExprStmt(Expr::Object(Vec::new()))],
            }],
        );
        let sites = JsAllocSites::of(&stmt);
        assert_eq!(sites.array, 1);
        assert_eq!(sites.object, 1);
        assert_eq!(sites.in_loop, 2);
        assert_eq!(sites.opaque, 0);
    }
}
