//! The JavaScript lowering of a JIR-R [`Func`]: v1's `base.rs`/`place.rs`/`rvalue.rs`/`ptr.rs`/
//! `value.rs`/`constant.rs`/`emit.rs` over `&TypeTable + &Interns` and W6b's [`Plan`] instead of
//! a `TyCtxt` and MIR.
//!
//! [`JsCx`] is the lowering context one function is lowered through. Everything is a method on
//! it so W8 (`call.rs`, `intrinsics.rs`, `unsize.rs`) and W9 (`host.rs`, `template.rs`) extend it
//! by adding modules, not by threading new state:
//!
//! - `expr_of(v)`: the JavaScript expression of an SSA value — the cached inline expression when
//!   the plan inlined it, its `let` name otherwise (`ty_of(v)` is its type);
//! - `cell_expr(c)`/`cell_ty(c)`: a cell's storage expression (`x` or `x[0]` when boxed);
//! - `walk_place`/`borrow_view`/`raw_view`/`read_place`/`write_place` (`place.rs`);
//! - `deref_read`/`deref_write`/`write_indirect`/`ptr_*`/`cast_pointer`/`build_fat` (`ptr.rs`);
//! - `binary`/`unary`/`cast`/`aggregate`/`repeat`/`discriminant`/`set_discriminant` (`rvalue.rs`);
//! - `const_expr`/`string_literal`/`caller_location_value` (`constant.rs`);
//! - `temp(value)`: a hoisted `$tN` temporary declared in the prelude;
//! - `zombie(message)`: records a zombie and returns the `__rt.js_abort` poison expression;
//! - `loc_of(origin)`: the `Stmt::Loc` marker of an origin, deduplicated as v1 `names.rs::Locs`;
//! - `decline(op, detail)`: the one way to say no (never panic, never emit wrong JavaScript).

pub mod adapters;
pub mod call;
pub mod constant;
pub mod emit;
pub mod function;
pub mod host;
pub mod i64_pair;
pub mod intrinsics;
pub mod place;
pub mod ptr;
pub mod rvalue;
pub mod template;
pub mod unsize;
pub mod value;

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};

use rcgjs_object::item::{FlatZombie, JsName, ZombieKind};
use rcgjs_object::jir::decline::{Decline, LowerDecline, OpClass};
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::origin::{OriginId, OriginTable, SourceFact, SourceFactId};
use rcgjs_object::jir::repr::{CellId, Func, Op, OpKind, Root, ValueId};
use rcgjs_object::jir::summary::ItemSummary;
use rcgjs_object::jir::types::{TypeId, TypeTable};
use rcgjs_object::jsast::{self, Expr, Loc, Stmt};
use rcgjs_object::opts::Opts;

use super::plan::Plan;

/// The lowering result type: a JavaScript value or a `Decline`.
pub type Res<T> = Result<T, Decline>;

/// The one constructor of a lowering decline.
pub fn decline(op: OpClass, detail: impl Into<String>) -> Decline {
    Decline::Lower(LowerDecline::Other {
        op,
        detail: detail.into(),
    })
}

/// The name of the trailing caller `Location` parameter (`intrinsics.rs:65`).
pub const CALLER_LOCATION_PARAM: &str = "$loc";

/// The state of one function's lowering.
pub struct JsCx<'a> {
    pub func: &'a Func,
    pub plan: &'a Plan,
    pub table: &'a TypeTable,
    pub interns: &'a Interns,
    pub opts: &'a Opts,
    pub summary: &'a ItemSummary,
    pub origins: Option<&'a OriginTable>,
    /// Every item of the program by its [`ItemId`], which is how a host lowering names the body
    /// of a closure it is handed (`host.rs`). Empty unless the driver supplied one.
    pub items: &'a host::ItemIndex,
    /// The drop glue of every type the program drops anywhere, which is how a host marker names
    /// the one-shot drop of a closure or a future it is handed (`host.rs`).
    pub drop_glue: &'a host::DropGlueIndex,
    /// Every vtable item of the program by the `(concrete, trait)` pair it is the vtable of,
    /// which is what an unsizing coercion to `dyn` names (`unsize.rs`). Empty unless the driver
    /// supplied one.
    pub vtables: &'a unsize::VtableIndex,
    /// The name of each spill register the flat direct-call ABI allocated, register 1 first
    /// (`crate::plan::abi_flat::AbiPlan::spill_names`). Empty unless the driver supplied one, in
    /// which case `emit::spill_name` names a register the way the plan would have preferred to.
    pub spills: &'a [JsName],
    /// The selected whole-program call ABI, when phase C applies one.
    pub abi: Option<&'a crate::plan::abi_flat::AbiPlan>,
    /// This item's value-local storage choices, when phase C applies them.
    pub physical: Option<(&'a JsName, &'a crate::plan::physical::PhysicalPlan)>,
    /// This declaration's selected physical signature. Unlike `abi.call_signature(item)`, this
    /// is absent on the canonical source when only its private `$n` body is physical.
    pub target_abi: Option<&'a crate::plan::abi_flat::FlatSig>,
    /// Private native-version targets selected for direct calls in this body.
    pub call_retargets: &'a crate::lower::CallRetargets,
    /// What one `view!` template instantiated in this body knows, by the value the
    /// `rcgjs.tc.template` call defined (`template.rs`).
    pub(super) instantiations: RefCell<BTreeMap<ValueId, template::Instantiation>>,
    /// The inline expression of every value the plan inlined, once its op has been lowered.
    exprs: RefCell<BTreeMap<ValueId, Expr>>,
    /// Normalized signed-`Int32` `(lo, hi)` pair expressions for 64-bit integer (`i64`/`u64`)
    /// parameters, inlined values, and cached temporaries.
    pair_exprs: RefCell<BTreeMap<ValueId, (Expr, Expr)>>,
    /// Carried 64-bit `*WithOverflow` results `(lo, hi, overflow_flag)` keyed by the pair value.
    pub(super) carried_i64_overflow: RefCell<BTreeMap<ValueId, (Expr, Expr, Expr)>>,
    /// Exact JavaScript formal parameter names in the emitted `function` signature (excluding
    /// expanded logical parameter names whose formal lanes are `<name>$lo`/`$buf`/etc.).
    formal_param_names: RefCell<HashSet<String>>,
    /// Tagged direct-call results kept as virtual `tag, payload...` products.
    ///
    /// Their expressions are canonical object *descriptions*, but are never assigned as objects:
    /// field/discriminant reads and prepared-call projections fold their members back to the
    /// captured scalar locals. Move/Copy aliases share the same description because tagged ABI
    /// admission has already proved every use field-only or a whole-value direct-call forward.
    tagged_exprs: RefCell<BTreeMap<ValueId, Expr>>,
    /// Allocation-bearing expressions paired with the JIR value that produced them. The final
    /// JavaScript allocation walker uses these only for provenance; they never affect emission.
    allocation_origins: RefCell<Vec<(ValueId, Expr)>>,
    /// The hoisted `$tN` temporaries this body has asked for, in allocation order.
    temps: RefCell<Vec<String>>,
    /// The zombies this body carries.
    zombies: RefCell<Vec<FlatZombie>>,
    /// The origin lowering is currently at, for zombies and source map markers.
    origin: Cell<Option<OriginId>>,
    /// The value the last returning block parked for the `Region::Return` that follows it.
    pending_return: RefCell<Option<Expr>>,
    /// The `(lo, hi)` pair parked for a `flat_abi` `i64_pair` `Region::Return`.
    pending_return_pair: RefCell<Option<(Expr, Expr)>>,
    /// The defining op of every value an op defines: `(block, index)`.
    defs: BTreeMap<ValueId, (u32, u32)>,
    /// Overflow pairs that are never built, and how each is spelled
    /// (`rvalue::carried_overflow_pairs`).
    carried_pairs: BTreeMap<ValueId, rvalue::CarriedPair>,
}

impl<'a> JsCx<'a> {
    pub fn new(
        func: &'a Func,
        plan: &'a Plan,
        table: &'a TypeTable,
        interns: &'a Interns,
        opts: &'a Opts,
        summary: &'a ItemSummary,
        origins: Option<&'a OriginTable>,
    ) -> Self {
        let mut defs = BTreeMap::new();
        for (b, block) in func.blocks.iter().enumerate() {
            for (i, op) in block.ops.iter().enumerate() {
                if let Some(result) = op.result {
                    defs.insert(result, (b as u32, i as u32));
                }
            }
        }
        JsCx {
            func,
            plan,
            table,
            interns,
            opts,
            summary,
            origins,
            items: host::EMPTY_ITEMS,
            drop_glue: host::EMPTY_DROP_GLUE,
            vtables: &unsize::EMPTY_VTABLES,
            spills: &[],
            abi: None,
            physical: None,
            target_abi: None,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
            instantiations: RefCell::new(BTreeMap::new()),
            exprs: RefCell::new(BTreeMap::new()),
            pair_exprs: RefCell::new(BTreeMap::new()),
            carried_i64_overflow: RefCell::new(BTreeMap::new()),
            formal_param_names: RefCell::new(HashSet::new()),
            tagged_exprs: RefCell::new(BTreeMap::new()),
            allocation_origins: RefCell::new(Vec::new()),
            temps: RefCell::new(Vec::new()),
            zombies: RefCell::new(Vec::new()),
            origin: Cell::new(None),
            pending_return: RefCell::new(None),
            pending_return_pair: RefCell::new(None),
            defs,
            carried_pairs: rvalue::carried_overflow_pairs(func, table),
        }
    }

    /// The same context, resolving closure bodies through the program's item index.
    pub fn with_items(mut self, items: &'a host::ItemIndex) -> Self {
        self.items = items;
        self
    }

    /// The same context, resolving a host marker's drop callbacks through the program's drop
    /// glue index.
    pub fn with_drop_glue(mut self, drop_glue: &'a host::DropGlueIndex) -> Self {
        self.drop_glue = drop_glue;
        self
    }

    /// The same context, resolving `dyn` coercions through the program's vtable index.
    pub fn with_vtables(mut self, vtables: &'a unsize::VtableIndex) -> Self {
        self.vtables = vtables;
        self
    }

    /// The same context, spelling spill registers the names the ABI plan allocated.
    pub fn with_spills(mut self, spills: &'a [JsName]) -> Self {
        self.spills = spills;
        self
    }

    /// The same context, with the selected physical direct-call ABI.
    pub fn with_abi(mut self, abi: Option<&'a crate::plan::abi_flat::AbiPlan>) -> Self {
        self.abi = abi;
        self
    }

    /// The same context, with linker-selected value-local storage for this declaration.
    pub fn with_physical(
        mut self,
        item: &'a JsName,
        physical: Option<&'a crate::plan::physical::PhysicalPlan>,
    ) -> Self {
        self.physical = physical.map(|physical| (item, physical));
        self
    }

    /// Exact numeric storage selected for one value in this declaration.
    pub(super) fn typed_storage(
        &self,
        mut value: ValueId,
    ) -> Option<crate::plan::physical::TypedStorage> {
        let (item, physical) = self.physical?;
        for _ in 0..=8 {
            if let Some(storage) = physical.typed_storage(item, value) {
                return Some(storage);
            }
            value = match &self.def_of(value)?.kind {
                OpKind::Move(source) | OpKind::Copy(source) | OpKind::Deref(source) => *source,
                OpKind::BorrowView {
                    root: Root::Value(source),
                    path,
                    ..
                } if path.is_empty() => *source,
                _ => return None,
            };
        }
        None
    }

    /// The physical signature of this exact declaration, if it is the selected private/in-place
    /// target rather than a canonical boundary body.
    pub fn with_target_abi(mut self, target: Option<&'a crate::plan::abi_flat::FlatSig>) -> Self {
        self.target_abi = target;
        self
    }

    /// The same context, with linker-selected private native call targets.
    pub fn with_call_retargets(mut self, call_retargets: &'a crate::lower::CallRetargets) -> Self {
        self.call_retargets = call_retargets;
        self
    }

    /// Gives a semantic value a physical JavaScript expression. Slice-lane parameters use this
    /// to remain one JIR value while their declaration carries three scalar lanes.
    pub(super) fn bind_expr(&self, value: ValueId, expression: Expr) {
        self.exprs.borrow_mut().insert(value, expression);
    }

    /// Gives a proven field-only tagged value its virtual canonical description.
    pub(super) fn bind_tagged_expr(&self, value: ValueId, expression: Expr) {
        self.exprs.borrow_mut().insert(value, expression.clone());
        self.tagged_exprs.borrow_mut().insert(value, expression);
    }

    /// Binds a 64-bit integer SSA value to its normalized signed-`Int32` `(lo, hi)` pair.
    pub(super) fn bind_pair(&self, value: ValueId, lo: Expr, hi: Expr) {
        self.pair_exprs.borrow_mut().insert(value, (lo, hi));
    }

    /// Records the exact JavaScript formal parameter names in the emitted signature.
    pub(super) fn set_formal_param_names(&self, names: HashSet<String>) {
        *self.formal_param_names.borrow_mut() = names;
    }

    /// Whether `name` is bound directly by a formal parameter in the emitted JS parameter list.
    pub(super) fn is_formal_param_name(&self, name: &str) -> bool {
        self.formal_param_names.borrow().contains(name)
    }

    /// Whether `ty` is a 64-bit integer lowered to signed-`Int32` `(lo, hi)` lanes.
    pub(super) fn is_i64_ty(&self, ty: TypeId) -> bool {
        i64_pair::is_i64(self.table, ty)
    }

    /// Whether `v`'s type is a 64-bit integer lowered to signed-`Int32` `(lo, hi)` lanes.
    pub(super) fn is_i64_value(&self, v: ValueId) -> bool {
        self.func.ty(v).is_some_and(|ty| self.is_i64_ty(ty))
    }

    /// Returns the low 32-bit signed-`Int32` value when `v` is defined by a constant scalar integer.
    pub(super) fn const_scalar_i32(&self, v: ValueId) -> Option<i32> {
        let mut cur = v;
        for _ in 0..8 {
            let def = self.def_of(cur)?;
            match &def.kind {
                OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Scalar { bits, .. }) => {
                    return Some((*bits as u32) as i32);
                }
                OpKind::Copy(src) | OpKind::Move(src) => cur = *src,
                _ => return None,
            }
        }
        None
    }

    /// Returns the normalized signed-`Int32` `(lo, hi)` expressions for a 64-bit SSA value.
    pub fn pair_of(&self, v: ValueId) -> Res<(Expr, Expr)> {
        if let Some(pair) = self.pair_exprs.borrow().get(&v) {
            return Ok(pair.clone());
        }
        if self.plan.inline.contains(&v) {
            let Some(def) = self.def_of(v) else {
                return Err(decline(
                    OpClass::Operand,
                    format!("inline 64-bit value {} has no defining op", v.0),
                ));
            };
            match &def.kind {
                OpKind::Const(tree) => {
                    if let Some(pair) = self.const_i64_pair(tree, self.ty_of(v)?) {
                        self.pair_exprs.borrow_mut().insert(v, pair.clone());
                        return Ok(pair);
                    }
                }
                OpKind::Copy(src) | OpKind::Move(src) => {
                    return self.pair_of(*src);
                }
                OpKind::Field {
                    base,
                    variant: None,
                    field: 0,
                } => {
                    let mut root = *base;
                    while let Some(def) = self.def_of(root) {
                        match &def.kind {
                            OpKind::Copy(src) | OpKind::Move(src) => root = *src,
                            _ => break,
                        }
                    }
                    if let Some((lo, hi, _)) = self.carried_i64_overflow.borrow().get(&root) {
                        return Ok((lo.clone(), hi.clone()));
                    }
                }
                _ => {}
            }
            let saved = self.origin.replace(Some(def.origin));
            let expr = self.op_expr(def, v);
            self.origin.set(saved);
            let expr = expr?;
            let pair = if i64_pair::is_trivial(&expr) {
                i64_pair::unpack_bigint_exprs(expr)
            } else {
                return Err(decline(
                    OpClass::Operand,
                    format!(
                        "inline 64-bit value {} requires statement lowering before pair_of",
                        v.0
                    ),
                ));
            };
            self.pair_exprs.borrow_mut().insert(v, pair.clone());
            return Ok(pair);
        }
        match self.plan.names.get(&v) {
            Some(name) => {
                let (lo, hi) = i64_pair::pair_names(name);
                Ok((jsast::id(lo), jsast::id(hi)))
            }
            None => Err(decline(
                OpClass::Operand,
                format!("64-bit value {} has neither a name nor a cached pair", v.0),
            )),
        }
    }

    /// The virtual tagged description already established for `value`.
    pub(super) fn tagged_expr(&self, value: ValueId) -> Option<Expr> {
        self.tagged_exprs.borrow().get(&value).cloned()
    }

    /// The op defining `v`, if an op (rather than a block parameter) does.
    pub fn def_of(&self, v: ValueId) -> Option<&'a Op> {
        let (block, index) = *self.defs.get(&v)?;
        self.func
            .blocks
            .get(block as usize)?
            .ops
            .get(index as usize)
    }

    /// The type of an SSA value.
    pub fn ty_of(&self, v: ValueId) -> Res<TypeId> {
        self.func.ty(v).ok_or_else(|| {
            decline(
                OpClass::Operand,
                format!("value {} is not in the function", v.0),
            )
        })
    }

    /// The JavaScript expression of an SSA value: the inline expression the plan chose, or the
    /// name the value was declared under.
    ///
    /// An inline value is lowered **lazily**, at its one use site: the plan promises the use is
    /// in the same block, after the definition, with only commuting ops in between, so evaluating
    /// there is evaluating in order. Laziness is what lets a synthetic view be spelled as the
    /// walked place by the `Deref`/`StoreThrough` that consumes it rather than as a pointer.
    pub fn expr_of(&self, v: ValueId) -> Res<Expr> {
        if let Some(expr) = self.exprs.borrow().get(&v) {
            return Ok(expr.clone());
        }
        if let Some(signed) = self
            .func
            .ty(v)
            .and_then(|ty| i64_pair::i64_signedness(self.table, ty))
        {
            let (lo, hi) = self.pair_of(v)?;
            let packed = i64_pair::pack_bigint(signed, lo, hi);
            if self.plan.inline.contains(&v) {
                self.exprs.borrow_mut().insert(v, packed.clone());
            }
            return Ok(packed);
        }
        // Every zero-sized value is `undefined`, whatever defined it and whatever it is called:
        // naming one would only spell `x = undefined; .. x ..` where v1 spells `undefined`.
        if self.table.is_zst(self.ty_of(v)?) {
            return Ok(jsast::undefined());
        }
        if self.plan.inline.contains(&v) {
            let Some(def) = self.def_of(v) else {
                return Err(decline(
                    OpClass::Operand,
                    format!("inline value {} has no defining op", v.0),
                ));
            };
            let saved = self.origin.replace(Some(def.origin));
            let expr = self.op_expr(def, v);
            self.origin.set(saved);
            let expr = expr?;
            self.record_allocation_origin(v, &expr);
            self.exprs.borrow_mut().insert(v, expr.clone());
            return Ok(expr);
        }
        match self.plan.names.get(&v) {
            Some(name) => Ok(jsast::id(name.clone())),
            None => Err(decline(
                OpClass::Operand,
                format!("value {} has neither a name nor an inline expression", v.0),
            )),
        }
    }

    /// Caches the expression of an inlined value, for a lowering that had to run statements at
    /// the definition site (a call whose prelude cannot be an expression, `emit.rs::op_stmts`).
    /// The use site then reads the value back rather than lowering the op a second time.
    pub(super) fn cache_expr(&self, v: ValueId, expr: Expr) {
        self.exprs.borrow_mut().insert(v, expr);
    }

    /// Records an expression when its defining JIR op is itself an allocation site.
    pub(super) fn record_allocation_origin(&self, value: ValueId, expression: &Expr) {
        let Some(op) = self.def_of(value) else { return };
        if crate::analysis::alloc_sites::classify(self.func, op, self.table).is_none() {
            return;
        }
        let mut origins = self.allocation_origins.borrow_mut();
        if !origins
            .iter()
            .any(|(seen, candidate)| *seen == value && candidate == expression)
        {
            origins.push((value, expression.clone()));
        }
    }

    /// Takes the allocation-expression provenance captured while emitting this item.
    pub(super) fn take_allocation_origins(&self) -> Vec<(ValueId, Expr)> {
        std::mem::take(&mut *self.allocation_origins.borrow_mut())
    }

    /// Whether the plan inlined `v` (its op becomes an expression consumed at its use site).
    pub fn is_inline(&self, v: ValueId) -> bool {
        self.plan.inline.contains(&v)
    }

    /// The `let` name of a value the plan named.
    pub fn name_of(&self, v: ValueId) -> Option<&'a str> {
        self.plan.names.get(&v).map(String::as_str)
    }

    /// The type of a cell.
    pub fn cell_ty(&self, cell: CellId) -> Res<TypeId> {
        self.func
            .cells
            .get(cell.0 as usize)
            .map(|def| def.ty)
            .ok_or_else(|| {
                decline(
                    OpClass::Place,
                    format!("cell {} is not in the function", cell.0),
                )
            })
    }

    /// Whether a cell is a `[v]` box.
    pub fn cell_boxed(&self, cell: CellId) -> bool {
        self.func
            .cells
            .get(cell.0 as usize)
            .is_some_and(|def| def.boxed)
    }

    /// The `let` name of a cell.
    pub fn cell_name(&self, cell: CellId) -> Res<&'a str> {
        self.plan
            .cell_names
            .get(&cell)
            .map(String::as_str)
            .ok_or_else(|| {
                decline(
                    OpClass::Place,
                    format!("cell {} has no name in the plan", cell.0),
                )
            })
    }

    /// How a cell is read and written: `x`, or `x[0]` when it is boxed (`place.rs::local_expr`).
    pub fn cell_expr(&self, cell: CellId) -> Res<Expr> {
        let name = jsast::id(self.cell_name(cell)?);
        Ok(if self.cell_boxed(cell) {
            jsast::index(name, jsast::num(0))
        } else {
            name
        })
    }

    /// Allocates a hoisted temporary holding `value` (`base.rs::temp`): the statement that fills
    /// it and the expression that reads it. Declared in the prelude.
    pub fn temp(&self, value: Expr) -> (Stmt, Expr) {
        let name = {
            let mut temps = self.temps.borrow_mut();
            let base = format!("$t{}", temps.len());
            let mut name = base.clone();
            let mut suffix = 0;
            while self.plan.globals.contains(&name) {
                suffix += 1;
                name = format!("{base}${suffix}");
            }
            temps.push(name.clone());
            name
        };
        (
            jsast::assign_stmt(jsast::id(name.clone()), value),
            jsast::id(name),
        )
    }

    /// The hoisted temporaries allocated so far.
    pub fn temps(&self) -> Vec<String> {
        self.temps.borrow().clone()
    }

    /// Sets the origin lowering is at.
    pub fn at(&self, origin: OriginId) {
        self.origin.set(Some(origin));
    }

    /// The source fact of an origin, when it has one.
    pub fn fact_of(&self, origin: OriginId) -> Option<&'a SourceFact> {
        let origin = self.origins?.origin(origin)?;
        self.interns.source_fact(origin.source?)
    }

    /// The source fact of the origin lowering is at, when it has one. What
    /// `intrinsics.rs::caller_location` spells `Location::caller()` from outside a
    /// `#[track_caller]` body.
    pub fn current_fact(&self) -> Option<&'a SourceFact> {
        self.fact_of(self.origin.get()?)
    }

    /// The `Loc` of an origin, spelled as `names.rs::location_of` does; `None` without a fact or
    /// with source maps off.
    pub fn loc_of(&self, origin: OriginId) -> Option<Loc> {
        let origin = self.origins?.origin(origin)?;
        self.loc_of_fact(origin.source?)
    }

    /// The `Loc` of an explicit source fact carried by a semantic payload.
    pub fn loc_of_fact(&self, source: SourceFactId) -> Option<Loc> {
        if !self.opts.source_map {
            return None;
        }
        let fact = self.interns.source_fact(source)?;
        let file = self.interns.str(fact.file)?;
        if file.is_empty() {
            return None;
        }
        Some(Loc {
            file: file.to_owned(),
            line: fact.line,
            col: fact.col,
        })
    }

    /// Records something the lowering cannot translate and stands in for it with a runtime abort
    /// (`base.rs::zombie`, `ZombieLog::record`).
    pub fn zombie(&self, message: String) -> Expr {
        let loc = self
            .origin
            .get()
            .and_then(|origin| self.fact_of(origin))
            .and_then(|fact| {
                let file = self.interns.str(fact.file)?;
                Some(format!("{file}:{}:{}", fact.line, fact.col + 1))
            })
            .unwrap_or_default();
        self.zombies.borrow_mut().push(FlatZombie {
            loc,
            message: message.clone(),
            kind: ZombieKind::Unsupported,
        });
        jsast::rt_call("js_abort", vec![jsast::string(message)])
    }

    /// The zombies recorded so far.
    pub fn take_zombies(&self) -> Vec<FlatZombie> {
        std::mem::take(&mut *self.zombies.borrow_mut())
    }
}
