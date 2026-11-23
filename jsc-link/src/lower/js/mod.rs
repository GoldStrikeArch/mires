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
    /// Whether this lowering hoists BigInt constants (`crate::lower::LowerInput::bigints`), and
    /// so widens an integer literal at compile time ([`JsCx::to_bigint`]).
    hoists_bigints: bool,
    /// Whether whole-program analysis determined that wide (`size > 1`) storage may be observed
    /// through 1-byte pointers/slices (`MIRES-008` lazy-proxy path).
    pub byte_views: crate::analysis::byte_views::ByteViewPlan,
    /// Whole-program Tier A typed-heap and `_boxes` tracking plan (`analysis::heap`).
    pub heap: Option<&'a crate::analysis::heap::HeapPlan>,
    /// Whether this function carries nested native Vec/String fields needing carrier-safe clone.
    pub nested_clone: crate::lower::NestedClone,
    /// Whole-program ASCII provenance facts for this function (`js-str=bytes`).
    ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'a>>,
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
    /// `js-str=bytes`: the `as_bytes()` views this body holds as the string itself
    /// ([`JsCx::byte_view`]). Computed on first use.
    byte_string_views: std::cell::OnceCell<std::collections::BTreeMap<ValueId, ByteView>>,
    /// Stage 3 (`js-str=bytes`): closed MIR-inlined `String.vec` operations in this body.
    pub(super) inlined_string_vec:
        std::cell::OnceCell<Option<crate::lower::semantic_ops::string::InlinedStringVecAnalysis>>,
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
    /// Unsigned 32-bit (`u32`/`usize`) SSA values whose uses all depend only on their low 32 bits
    /// (bitwise, shifts, 32-bit wrapping arithmetic, narrow casts), so they may stay in signed
    /// `Int32` without a trailing `>>> 0`.
    int32_only: std::collections::BTreeSet<ValueId>,
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
        let carried_pairs = rvalue::carried_overflow_pairs(func, table);
        let int32_only = rvalue::int32_only_values(func, table, interns, &carried_pairs);
        let inlined_string_vec = std::cell::OnceCell::new();
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
            hoists_bigints: false,
            byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
            heap: None,
            nested_clone: crate::lower::NestedClone::default(),
            ascii: None,
            instantiations: RefCell::new(BTreeMap::new()),
            exprs: RefCell::new(BTreeMap::new()),
            pair_exprs: RefCell::new(BTreeMap::new()),
            carried_i64_overflow: RefCell::new(BTreeMap::new()),
            formal_param_names: RefCell::new(HashSet::new()),
            tagged_exprs: RefCell::new(BTreeMap::new()),
            byte_string_views: std::cell::OnceCell::new(),
            inlined_string_vec,
            allocation_origins: RefCell::new(Vec::new()),
            temps: RefCell::new(Vec::new()),
            zombies: RefCell::new(Vec::new()),
            origin: Cell::new(None),
            pending_return: RefCell::new(None),
            pending_return_pair: RefCell::new(None),
            defs,
            carried_pairs,
            int32_only,
        }
    }

    /// Whether `value` is a 32-bit unsigned SSA value whose consumers all accept signed `Int32`.
    pub(super) fn is_int32_only(&self, value: ValueId) -> bool {
        self.int32_only.contains(&value)
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

    /// The same context, carrying whether wide byte views are reachable in the program.
    pub fn with_byte_views(
        mut self,
        byte_views: crate::analysis::byte_views::ByteViewPlan,
    ) -> Self {
        self.byte_views = byte_views;
        self
    }

    /// The same context, carrying the whole-program Tier A typed-heap and `_boxes` plan.
    pub fn with_heap(mut self, heap: Option<&'a crate::analysis::heap::HeapPlan>) -> Self {
        self.heap = heap;
        self
    }

    /// Returns the active whole-program [`crate::analysis::heap::HeapPlan`] (conservative default
    /// when `None`).
    pub fn heap(&self) -> &crate::analysis::heap::HeapPlan {
        self.heap
            .unwrap_or(&crate::analysis::heap::DEFAULT_HEAP_PLAN)
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

    /// The same context, lowering for a declaration whose BigInt constants are hoisted.
    pub fn with_bigint_hoist(mut self, hoists: bool) -> Self {
        self.hoists_bigints = hoists;
        self
    }

    /// Whether this lowering hoists BigInt constants.
    pub fn hoists_bigints(&self) -> bool {
        self.hoists_bigints
    }

    /// The same context, carrying per-function nested native carrier clone policy.
    pub fn with_nested_clone(mut self, nested_clone: crate::lower::NestedClone) -> Self {
        self.nested_clone = nested_clone;
        self
    }

    /// The same context, carrying whole-program ASCII facts (`js-str=bytes`).
    pub fn with_ascii(mut self, ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'a>>) -> Self {
        self.ascii = ascii;
        self
    }

    /// Whole-program ASCII facts for this item, when available.
    pub fn ascii(&self) -> Option<crate::analysis::ascii::FuncAsciiFacts<'a>> {
        self.ascii
    }

    /// Whether `value` is statically proven to hold only ASCII bytes (`b < 0x80`).
    pub fn is_known_ascii(&self, value: ValueId) -> bool {
        self.ascii.is_some_and(|ascii| ascii.is_known_ascii(value))
    }

    /// `BigInt(value)`, the widening of a number to a BigInt (`jsast::to_bigint`). When this
    /// lowering hoists BigInt constants and `value` is an integer literal, the widening is done
    /// here instead: the result is the BigInt literal, which the hoist then names like any other
    /// constant. Without the hoist the call is kept, so `js-ecma-target=latest` and the cost trials
    /// print what they always printed.
    pub fn to_bigint(&self, value: Expr) -> Expr {
        match crate::lower::bigint::integer_literal(&value) {
            Some(integer) if self.hoists_bigints => jsast::bigint(integer),
            _ => jsast::to_bigint(value),
        }
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

    /// `js-str=bytes`: whether `v` is a closed byte-string view over `&str` / `&String`.
    pub fn byte_view(&self, v: ValueId) -> Option<ByteView> {
        if !crate::lower::bstr::enabled() {
            return None;
        }
        self.byte_string_views
            .get_or_init(|| self.find_byte_string_views())
            .get(&v)
            .copied()
    }

    pub fn byte_string_view(&self, v: ValueId) -> bool {
        self.byte_view(v)
            .is_some_and(|view| view.mode == ByteViewMode::FullSlice)
    }

    pub(super) fn inlined_string_vec(
        &self,
    ) -> Option<&crate::lower::semantic_ops::string::InlinedStringVecAnalysis> {
        self.inlined_string_vec
            .get_or_init(|| {
                if crate::lower::bstr::enabled() {
                    crate::lower::semantic_ops::string::analyze_inlined_string_vec(
                        self.func,
                        self.table,
                        self.interns,
                        |item, link_name| {
                            crate::lower::semantic_ops::string::is_raw_vec_reserve_by_link_name(
                                self.interns,
                                Some(self.items),
                                item,
                                link_name,
                            )
                        },
                    )
                } else {
                    None
                }
            })
            .as_ref()
    }

    fn find_byte_string_views(&self) -> std::collections::BTreeMap<ValueId, ByteView> {
        byte_view_analysis::analyze(
            self.func,
            self.plan,
            self.table,
            self.interns,
            self.inlined_string_vec(),
        )
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteViewMode {
    /// `&[u8]` / `*const [u8]` / `*const str` holding the full JS string (`off == 0`,
    /// `len == base.length`) directly as a plain JS `string`.
    FullSlice,
    /// `&[u8]` / `*const [u8]` holding a `(buf, off, len)` subslice.
    VarSlice,
    /// Thin byte pointer (`*const u8` / `&u8` / `*const ()`), held as a plain JS `number`
    /// byte offset into `base_str`.
    ThinPtr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteView {
    pub mode: ByteViewMode,
    /// Single dominating `&str` (or `&String` for StringBuilder/HostString `as_bytes`)
    /// when unique in the connected component; `None` for cell-rooted or multi-root `FullSlice`.
    pub base_str: Option<ValueId>,
    /// Single dominating `String` cell for inlined `String::as_bytes`, when rooted at a `Cell`.
    pub base_cell: Option<rcgjs_object::jir::repr::CellId>,
    /// True if base is `StringBuilder` (`{ text, cap, len }`) rather than `&str` / `HostString`.
    pub base_is_string_obj: bool,
    /// Slice `ValueId` from which a `[Deref, Index | ConstantIndex]` borrow originates.
    pub borrow_slice: Option<ValueId>,
}

mod byte_view_analysis {
    use std::collections::{BTreeMap, BTreeSet};

    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::repr::{
        Carrier, CellId, Dominators, Func, OpKind, Proj, RBlockId, RTerm, Root, ValueId,
    };
    use rcgjs_object::jir::semantic::{BinaryOp, CastKind, UnaryOp};
    use rcgjs_object::jir::types::{TypeId, TypeKind, TypeTable};

    use super::{value, ByteView, ByteViewMode};
    use crate::lower::plan::Plan;
    use crate::lower::semantic_ops::string::{InlinedStringVecAnalysis, StringRoot};

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
    enum BaseKey {
        Value(ValueId),
        Cell(CellId),
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ByteKind {
        Slice,
        ThinPtr,
    }

    struct UnionFind {
        parent: Vec<usize>,
    }

    impl UnionFind {
        fn new(n: usize) -> Self {
            Self {
                parent: (0..n).collect(),
            }
        }

        fn find(&mut self, mut x: usize) -> usize {
            while self.parent[x] != x {
                let p = self.parent[x];
                self.parent[x] = self.parent[p];
                x = p;
            }
            x
        }

        fn union(&mut self, a: usize, b: usize) {
            let ra = self.find(a);
            let rb = self.find(b);
            if ra != rb {
                self.parent[rb] = ra;
            }
        }
    }

    fn is_immutable_str_ref(table: &TypeTable, ty: TypeId) -> bool {
        matches!(
            table.get(table.peeled(ty)).map(|d| &d.kind),
            Some(TypeKind::Reference {
                pointee,
                mutable: false,
            }) if matches!(value::kind(table, *pointee), Some(TypeKind::Str))
        )
    }

    fn is_const_str_raw(table: &TypeTable, ty: TypeId) -> bool {
        matches!(
            table.get(table.peeled(ty)).map(|d| &d.kind),
            Some(TypeKind::RawPointer {
                pointee,
                mutable: false,
            }) if matches!(value::kind(table, *pointee), Some(TypeKind::Str))
        )
    }

    fn is_immutable_byte_slice(table: &TypeTable, ty: TypeId) -> bool {
        matches!(
            table.get(table.peeled(ty)).map(|d| &d.kind),
            Some(
                TypeKind::Reference {
                    pointee,
                    mutable: false,
                } | TypeKind::RawPointer {
                    pointee,
                    mutable: false,
                }
            ) if matches!(
                value::kind(table, *pointee),
                Some(TypeKind::Slice(element)) if value::is_u8(table, *element)
            )
        )
    }

    fn is_immutable_byte_ptr(table: &TypeTable, ty: TypeId) -> bool {
        matches!(
            table.get(table.peeled(ty)).map(|d| &d.kind),
            Some(
                TypeKind::Reference {
                    pointee,
                    mutable: false,
                } | TypeKind::RawPointer {
                    pointee,
                    mutable: false,
                }
            ) if value::is_u8(table, *pointee)
        )
    }

    fn is_const_unit_ptr(table: &TypeTable, ty: TypeId) -> bool {
        matches!(
            table.get(table.peeled(ty)).map(|d| &d.kind),
            Some(TypeKind::RawPointer {
                pointee,
                mutable: false,
            }) if table.is_zst(*pointee)
        )
    }

    fn is_slice_ty(table: &TypeTable, ty: TypeId) -> bool {
        is_immutable_byte_slice(table, ty) || is_const_str_raw(table, ty)
    }

    fn is_ptr_ty(table: &TypeTable, ty: TypeId) -> bool {
        is_immutable_byte_ptr(table, ty) || is_const_unit_ptr(table, ty)
    }

    fn classify_ty(table: &TypeTable, ty: TypeId) -> Option<ByteKind> {
        if is_slice_ty(table, ty) {
            Some(ByteKind::Slice)
        } else if is_ptr_ty(table, ty) {
            Some(ByteKind::ThinPtr)
        } else {
            None
        }
    }

    fn is_byte_view_candidate(func: &Func, table: &TypeTable, v: ValueId) -> bool {
        func.ty(v).and_then(|ty| classify_ty(table, ty)).is_some()
    }

    fn is_reusable_base_str(func: &Func, plan: &Plan, mut base: ValueId) -> Option<ValueId> {
        let mut seen = BTreeSet::new();
        while plan.inline.contains(&base) {
            if !seen.insert(base) {
                return None;
            }
            let mut next = None;
            for block in &func.blocks {
                for op in &block.ops {
                    if op.result == Some(base) {
                        match &op.kind {
                            OpKind::Const(_) => return Some(base),
                            OpKind::Move(src) | OpKind::Copy(src) => next = Some(*src),
                            _ => return None,
                        }
                    }
                }
            }
            match next {
                Some(src) => base = src,
                None => return Some(base),
            }
        }
        Some(base)
    }

    pub(super) fn analyze(
        func: &Func,
        plan: &Plan,
        table: &TypeTable,
        interns: &Interns,
        inlined_vec: Option<&InlinedStringVecAnalysis>,
    ) -> BTreeMap<ValueId, ByteView> {
        let n = func.values.len();
        if n == 0 {
            return BTreeMap::new();
        }
        let block_index: BTreeMap<RBlockId, usize> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(idx, block)| (block.id, idx))
            .collect();
        type ByteViewRoot = (BaseKey, bool, Option<ValueId>, ByteKind);
        let mut uf = UnionFind::new(n);
        let mut slice_uf = UnionFind::new(n);
        let mut roots: Vec<Option<ByteViewRoot>> = vec![None; n];
        let mut valid_op_def = vec![false; n];
        let mut poisoned = vec![false; n];
        let mut var_slice_seed = vec![false; n];
        let mut borrow_slice: Vec<Option<ValueId>> = vec![None; n];
        let mut def_block: Vec<Option<RBlockId>> = vec![None; n];
        let mut use_blocks: Vec<Vec<RBlockId>> = vec![Vec::new(); n];
        let mut param_has_pred = vec![false; n];
        let mut is_block_param = vec![false; n];
        let mut has_non_trivial_thin_use = vec![false; n];

        for (bi, block) in func.blocks.iter().enumerate() {
            for &param in &block.params {
                let idx = param.0 as usize;
                def_block[idx] = Some(block.id);
                if bi > 0 && is_byte_view_candidate(func, table, param) {
                    is_block_param[idx] = true;
                }
            }
            for op in &block.ops {
                if let Some(res) = op.result {
                    def_block[res.0 as usize] = Some(block.id);
                }
                for u in op.kind.uses() {
                    use_blocks[u.0 as usize].push(block.id);
                }
            }
            for u in block.term.uses() {
                use_blocks[u.0 as usize].push(block.id);
            }
        }

        for block in &func.blocks {
            for op in &block.ops {
                let Some(res) = op.result else {
                    for u in op.kind.uses() {
                        if is_byte_view_candidate(func, table, u) {
                            poisoned[u.0 as usize] = true;
                        }
                    }
                    continue;
                };
                let r = res.0 as usize;
                let res_kind = func.ty(res).and_then(|ty| classify_ty(table, ty));

                let mut handled = false;
                if let Some(analysis) = inlined_vec {
                    if let Some(&cid) = analysis.value_cluster.get(&res) {
                        let cluster = &analysis.clusters[cid];
                        let witness_carrier = cluster
                            .witness_values
                            .first()
                            .map(|w| func.values[w.0 as usize].carrier);
                        if cluster.slice_values.contains(&res)
                            && res_kind == Some(ByteKind::Slice)
                            && matches!(
                                witness_carrier,
                                Some(Carrier::HostString | Carrier::StringBuilder)
                            )
                        {
                            let is_str_obj = witness_carrier == Some(Carrier::StringBuilder);
                            let (key, seed_op) = match cluster.root {
                                StringRoot::Value(v) => (BaseKey::Value(v), None),
                                StringRoot::Cell(c) => (BaseKey::Cell(c), Some(res)),
                            };
                            roots[r] = Some((key, is_str_obj, seed_op, ByteKind::Slice));
                            valid_op_def[r] = true;
                            handled = true;
                        }
                    }
                }
                if !handled {
                    match &op.kind {
                        OpKind::Cast {
                            kind:
                                CastKind::Transmute | CastKind::PtrToPtr | CastKind::PointerCoercion,
                            v: src,
                            from,
                            to,
                        } => {
                            if is_immutable_str_ref(table, *from) {
                                if let Some(k) = classify_ty(table, *to) {
                                    roots[r] = Some((BaseKey::Value(*src), false, None, k));
                                    valid_op_def[r] = true;
                                    if k == ByteKind::ThinPtr {
                                        has_non_trivial_thin_use[r] = true;
                                    }
                                    handled = true;
                                }
                            } else if let (Some(sk), Some(rk)) =
                                (classify_ty(table, *from), res_kind)
                            {
                                if sk == ByteKind::Slice || rk == ByteKind::ThinPtr {
                                    let s = src.0 as usize;
                                    uf.union(s, r);
                                    valid_op_def[r] = true;
                                    if sk == ByteKind::Slice && rk == ByteKind::Slice {
                                        slice_uf.union(s, r);
                                    } else {
                                        has_non_trivial_thin_use[r] = true;
                                    }
                                    handled = true;
                                }
                            }
                        }
                        OpKind::RawView {
                            root: Root::Value(src),
                            path,
                            mutable: false,
                        } if path.as_slice() == [Proj::Deref]
                            && func
                                .ty(*src)
                                .is_some_and(|ty| is_immutable_str_ref(table, ty))
                            && func.ty(res).is_some_and(|ty| is_const_str_raw(table, ty)) =>
                        {
                            roots[r] = Some((BaseKey::Value(*src), false, None, ByteKind::Slice));
                            valid_op_def[r] = true;
                            handled = true;
                        }
                        OpKind::Move(src) | OpKind::Copy(src) => {
                            let sk = func.ty(*src).and_then(|ty| classify_ty(table, ty));
                            if let (Some(sk), Some(rk)) = (sk, res_kind) {
                                if sk == rk {
                                    let s = src.0 as usize;
                                    uf.union(s, r);
                                    valid_op_def[r] = true;
                                    if sk == ByteKind::Slice {
                                        slice_uf.union(s, r);
                                    } else {
                                        has_non_trivial_thin_use[s] = true;
                                        has_non_trivial_thin_use[r] = true;
                                    }
                                    handled = true;
                                }
                            }
                        }
                        OpKind::BorrowView {
                            root: Root::Value(src),
                            path,
                            mutable: false,
                        }
                        | OpKind::RawView {
                            root: Root::Value(src),
                            path,
                            mutable: false,
                        } => {
                            if let (Some(sk), Some(rk)) = (
                                func.ty(*src).and_then(|ty| classify_ty(table, ty)),
                                res_kind,
                            ) {
                                let s = src.0 as usize;
                                match path.as_slice() {
                                    [Proj::Deref] if sk == rk => {
                                        uf.union(s, r);
                                        valid_op_def[r] = true;
                                        if sk == ByteKind::Slice {
                                            slice_uf.union(s, r);
                                        } else {
                                            has_non_trivial_thin_use[s] = true;
                                            has_non_trivial_thin_use[r] = true;
                                        }
                                        handled = true;
                                    }
                                    [Proj::Deref, Proj::Index(_) | Proj::ConstantIndex { .. }]
                                        if sk == ByteKind::Slice
                                            && func.ty(res).is_some_and(|ty| {
                                                is_immutable_byte_ptr(table, ty)
                                            }) =>
                                    {
                                        uf.union(s, r);
                                        valid_op_def[r] = true;
                                        borrow_slice[r] = Some(*src);
                                        handled = true;
                                    }
                                    [Proj::Deref, Proj::Subslice { .. }]
                                        if sk == ByteKind::Slice && rk == ByteKind::Slice =>
                                    {
                                        uf.union(s, r);
                                        valid_op_def[r] = true;
                                        var_slice_seed[r] = true;
                                        handled = true;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        OpKind::SliceView { ptr, .. } => {
                            let pk = func.ty(*ptr).and_then(|ty| classify_ty(table, ty));
                            if pk.is_some() && res_kind == Some(ByteKind::Slice) {
                                let p = ptr.0 as usize;
                                uf.union(p, r);
                                valid_op_def[r] = true;
                                var_slice_seed[r] = true;
                                has_non_trivial_thin_use[p] = true;
                                handled = true;
                            }
                        }
                        OpKind::Binary {
                            op: BinaryOp::Offset,
                            l,
                            ty,
                            ..
                        } => {
                            if is_immutable_byte_ptr(table, *ty)
                                && func
                                    .ty(*l)
                                    .is_some_and(|lt| is_immutable_byte_ptr(table, lt))
                                && func
                                    .ty(res)
                                    .is_some_and(|rt| is_immutable_byte_ptr(table, rt))
                            {
                                let li = l.0 as usize;
                                uf.union(li, r);
                                valid_op_def[r] = true;
                                has_non_trivial_thin_use[li] = true;
                                has_non_trivial_thin_use[r] = true;
                                handled = true;
                            }
                        }
                        OpKind::Binary {
                            op:
                                BinaryOp::Eq
                                | BinaryOp::Ne
                                | BinaryOp::Lt
                                | BinaryOp::Le
                                | BinaryOp::Gt
                                | BinaryOp::Ge,
                            l,
                            r: rhs,
                            ty,
                        } => {
                            if is_ptr_ty(table, *ty) {
                                uf.union(l.0 as usize, rhs.0 as usize);
                                has_non_trivial_thin_use[l.0 as usize] = true;
                                has_non_trivial_thin_use[rhs.0 as usize] = true;
                                handled = true;
                            } else if !is_slice_ty(table, *ty)
                                && !is_byte_view_candidate(func, table, *l)
                                && !is_byte_view_candidate(func, table, *rhs)
                            {
                                handled = true;
                            }
                        }
                        OpKind::Unary {
                            op: UnaryOp::PtrMetadata,
                            v,
                            ..
                        } => {
                            if func.ty(*v).is_some_and(|ty| is_slice_ty(table, ty)) {
                                handled = true;
                            }
                        }
                        OpKind::Deref(ptr) => {
                            if func
                                .ty(*ptr)
                                .is_some_and(|ty| is_immutable_byte_ptr(table, ty))
                            {
                                handled = true;
                            }
                        }
                        OpKind::Intrinsic {
                            name, types, args, ..
                        } => {
                            let name_s = interns.str(*name).unwrap_or("");
                            match name_s {
                                "transmute" | "transmute_unchecked"
                                    if types.len() == 2 && args.len() == 1 =>
                                {
                                    let (from, to, src) = (types[0], types[1], args[0]);
                                    if is_immutable_str_ref(table, from) {
                                        if let Some(k) = classify_ty(table, to) {
                                            roots[r] = Some((BaseKey::Value(src), false, None, k));
                                            valid_op_def[r] = true;
                                            if k == ByteKind::ThinPtr {
                                                has_non_trivial_thin_use[r] = true;
                                            }
                                            handled = true;
                                        }
                                    } else if let (Some(sk), Some(rk)) =
                                        (classify_ty(table, from), res_kind)
                                    {
                                        if sk == ByteKind::Slice || rk == ByteKind::ThinPtr {
                                            let s = src.0 as usize;
                                            uf.union(s, r);
                                            valid_op_def[r] = true;
                                            if sk == ByteKind::Slice && rk == ByteKind::Slice {
                                                slice_uf.union(s, r);
                                            } else {
                                                has_non_trivial_thin_use[r] = true;
                                            }
                                            handled = true;
                                        }
                                    }
                                }
                                "offset" | "arith_offset"
                                    if args.len() == 2
                                        && types
                                            .first()
                                            .is_some_and(|t| value::is_u8(table, *t))
                                        && func
                                            .ty(args[0])
                                            .is_some_and(|lt| is_immutable_byte_ptr(table, lt))
                                        && func
                                            .ty(res)
                                            .is_some_and(|rt| is_immutable_byte_ptr(table, rt)) =>
                                {
                                    let li = args[0].0 as usize;
                                    uf.union(li, r);
                                    valid_op_def[r] = true;
                                    has_non_trivial_thin_use[li] = true;
                                    has_non_trivial_thin_use[r] = true;
                                    handled = true;
                                }
                                "ptr_offset_from" | "ptr_offset_from_unsigned"
                                    if args.len() == 2
                                        && types
                                            .first()
                                            .is_some_and(|t| value::is_u8(table, *t))
                                        && func
                                            .ty(args[0])
                                            .is_some_and(|lt| is_immutable_byte_ptr(table, lt))
                                        && func
                                            .ty(args[1])
                                            .is_some_and(|rt| is_immutable_byte_ptr(table, rt)) =>
                                {
                                    uf.union(args[0].0 as usize, args[1].0 as usize);
                                    has_non_trivial_thin_use[args[0].0 as usize] = true;
                                    has_non_trivial_thin_use[args[1].0 as usize] = true;
                                    handled = true;
                                }
                                "read_via_copy" | "volatile_load" | "unaligned_volatile_load"
                                    if args.len() == 1
                                        && types
                                            .first()
                                            .is_some_and(|t| value::is_u8(table, *t))
                                        && func
                                            .ty(args[0])
                                            .is_some_and(|lt| is_immutable_byte_ptr(table, lt)) =>
                                {
                                    has_non_trivial_thin_use[args[0].0 as usize] = true;
                                    handled = true;
                                }
                                "ptr_metadata"
                                    if args.len() == 1
                                        && func
                                            .ty(args[0])
                                            .is_some_and(|ty| is_slice_ty(table, ty)) =>
                                {
                                    handled = true;
                                }
                                "slice_get_unchecked"
                                    if args.len() == 2
                                        && func.ty(args[0]).is_some_and(|ty| {
                                            is_immutable_byte_slice(table, ty)
                                        })
                                        && func
                                            .ty(res)
                                            .is_some_and(|ty| is_immutable_byte_ptr(table, ty)) =>
                                {
                                    let s = args[0].0 as usize;
                                    uf.union(s, r);
                                    valid_op_def[r] = true;
                                    has_non_trivial_thin_use[r] = true;
                                    handled = true;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                } // end if !handled

                if !handled {
                    for u in op.kind.uses() {
                        if is_byte_view_candidate(func, table, u) {
                            poisoned[u.0 as usize] = true;
                        }
                    }
                    if res_kind.is_some() {
                        poisoned[r] = true;
                    }
                }
            }

            let mut check_edge = |target: RBlockId, args: &[ValueId]| {
                let Some(&bi) = block_index.get(&target) else {
                    for &arg in args {
                        if is_byte_view_candidate(func, table, arg) {
                            poisoned[arg.0 as usize] = true;
                        }
                    }
                    return;
                };
                for (pos, &arg) in args.iter().enumerate() {
                    let Some(&param) = func.blocks[bi].params.get(pos) else {
                        if is_byte_view_candidate(func, table, arg) {
                            poisoned[arg.0 as usize] = true;
                        }
                        continue;
                    };
                    connect_edge(
                        func,
                        table,
                        &mut uf,
                        &mut slice_uf,
                        &mut poisoned,
                        &mut param_has_pred,
                        &mut has_non_trivial_thin_use,
                        arg,
                        param,
                    );
                }
            };

            match &block.term {
                RTerm::Goto(target, args) => check_edge(*target, args),
                RTerm::Switch {
                    scrut,
                    cases,
                    default,
                    ..
                } => {
                    for (_, target, args) in cases {
                        check_edge(*target, args);
                    }
                    check_edge(default.0, &default.1);
                    if is_byte_view_candidate(func, table, *scrut) {
                        poisoned[scrut.0 as usize] = true;
                    }
                }
                RTerm::Invoke { normal, unwind } => {
                    check_edge(normal.0, &normal.1);
                    check_edge(unwind.0, &unwind.1);
                }
                RTerm::Return(Some(v)) | RTerm::Throw(Some(v)) => {
                    if is_byte_view_candidate(func, table, *v) {
                        poisoned[v.0 as usize] = true;
                    }
                }
                RTerm::Return(None) | RTerm::Throw(None) | RTerm::Unreachable => {}
            }
        }

        for v in &plan.physical_slices {
            if let Some(slot) = var_slice_seed.get_mut(v.0 as usize) {
                *slot = true;
            }
        }

        let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, root) in roots.iter().enumerate() {
            if root.is_some() {
                comps.entry(uf.find(i)).or_default();
            }
        }
        if comps.is_empty() {
            return BTreeMap::new();
        }
        for i in 0..n {
            let comp = uf.find(i);
            if let Some(members) = comps.get_mut(&comp) {
                members.push(i);
            }
        }

        let mut var_slice_roots = BTreeSet::new();
        for (i, &is_seed) in var_slice_seed.iter().enumerate() {
            if is_seed {
                var_slice_roots.insert(slice_uf.find(i));
            }
        }

        let doms = Dominators::of(func);
        let mut out = BTreeMap::new();

        for members in comps.into_values() {
            let mut disqualified = false;
            let mut comp_roots: BTreeMap<(BaseKey, bool), Option<ValueId>> = BTreeMap::new();
            let mut needs_single_base = false;

            for &v in &members {
                if poisoned[v] {
                    disqualified = true;
                    break;
                }
                let ok_def = valid_op_def[v] || (is_block_param[v] && param_has_pred[v]);
                if !ok_def {
                    disqualified = true;
                    break;
                }
                let Some(kind) = func
                    .ty(ValueId(v as u32))
                    .and_then(|ty| classify_ty(table, ty))
                else {
                    disqualified = true;
                    break;
                };
                if let Some((key, is_str_obj, seed_op, _)) = roots[v] {
                    comp_roots
                        .entry((key, is_str_obj))
                        .and_modify(|prev| {
                            if *prev != seed_op {
                                *prev = None;
                            }
                        })
                        .or_insert(seed_op);
                }
                if kind == ByteKind::Slice && var_slice_roots.contains(&slice_uf.find(v)) {
                    needs_single_base = true;
                }
                if kind == ByteKind::ThinPtr
                    && (has_non_trivial_thin_use[v] || borrow_slice[v].is_none())
                {
                    needs_single_base = true;
                }
            }
            if disqualified {
                continue;
            }

            let single_base = if needs_single_base || comp_roots.len() == 1 {
                if comp_roots.len() != 1 {
                    continue;
                }
                let (&(raw_key, is_str_obj), &seed_op) = comp_roots.iter().next().unwrap();
                let (base_val, base_cell, dom_val) = match raw_key {
                    BaseKey::Value(raw_base) => {
                        let base = if needs_single_base {
                            let Some(reusable) = is_reusable_base_str(func, plan, raw_base) else {
                                continue;
                            };
                            reusable
                        } else {
                            raw_base
                        };
                        (Some(base), None, Some(base))
                    }
                    BaseKey::Cell(cell) => (None, Some(cell), seed_op),
                };
                if needs_single_base {
                    let Some(dom_v) = dom_val else {
                        continue;
                    };
                    let Some(base_blk) = def_block[dom_v.0 as usize] else {
                        continue;
                    };
                    let mut dom_ok = true;
                    for &v in &members {
                        if let Some(d_blk) = def_block[v] {
                            if !doms.dominates(base_blk, d_blk) {
                                dom_ok = false;
                                break;
                            }
                        }
                        for &u_blk in &use_blocks[v] {
                            if !doms.dominates(base_blk, u_blk) {
                                dom_ok = false;
                                break;
                            }
                        }
                        if !dom_ok {
                            break;
                        }
                    }
                    if !dom_ok {
                        continue;
                    }
                }
                Some((base_val, base_cell, is_str_obj))
            } else {
                None
            };

            for &v in &members {
                let kind = func
                    .ty(ValueId(v as u32))
                    .and_then(|ty| classify_ty(table, ty))
                    .unwrap();
                let mode = match kind {
                    ByteKind::Slice => {
                        if var_slice_roots.contains(&slice_uf.find(v)) {
                            ByteViewMode::VarSlice
                        } else {
                            ByteViewMode::FullSlice
                        }
                    }
                    ByteKind::ThinPtr => ByteViewMode::ThinPtr,
                };
                out.insert(
                    ValueId(v as u32),
                    ByteView {
                        mode,
                        base_str: single_base.and_then(|(b, _, _)| b),
                        base_cell: single_base.and_then(|(_, c, _)| c),
                        base_is_string_obj: single_base.is_some_and(|(_, _, obj)| obj),
                        borrow_slice: borrow_slice[v],
                    },
                );
            }
        }

        out
    }

    #[allow(clippy::too_many_arguments)]
    fn connect_edge(
        func: &Func,
        table: &TypeTable,
        uf: &mut UnionFind,
        slice_uf: &mut UnionFind,
        poisoned: &mut [bool],
        param_has_pred: &mut [bool],
        has_non_trivial_thin_use: &mut [bool],
        arg: ValueId,
        param: ValueId,
    ) {
        let a = arg.0 as usize;
        let p = param.0 as usize;
        let ak = func.ty(arg).and_then(|ty| classify_ty(table, ty));
        let pk = func.ty(param).and_then(|ty| classify_ty(table, ty));
        if ak.is_none() && pk.is_none() {
            return;
        }
        param_has_pred[p] = true;
        match (ak, pk) {
            (Some(ak), Some(pk)) if ak == pk => {
                uf.union(a, p);
                if ak == ByteKind::Slice {
                    slice_uf.union(a, p);
                } else {
                    has_non_trivial_thin_use[a] = true;
                    has_non_trivial_thin_use[p] = true;
                }
            }
            _ => {
                poisoned[a] = true;
                poisoned[p] = true;
            }
        }
    }
}

#[cfg(test)]
mod byte_view_tests {
    use super::*;
    use crate::lower::plan::NO_ORIGIN;
    use rcgjs_object::jir::interns::Interns;
    use rcgjs_object::jir::repr::{FuncBuilder, OpKind, Proj, RTerm, Root};
    use rcgjs_object::jir::semantic::{
        AbiSignature, BinaryOp, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, ItemId,
        Signature, UnaryOp,
    };
    use rcgjs_object::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, PointerForm, TypeDescriptor, TypeId, TypeKind,
        TypeMeta,
    };
    use rcgjs_object::opts::Opts;

    const UNIT: TypeId = TypeId(1);
    const BOOL: TypeId = TypeId(2);
    const U8: TypeId = TypeId(3);
    const USIZE: TypeId = TypeId(4);
    const ISIZE: TypeId = TypeId(5);
    const STR: TypeId = TypeId(6);
    const STR_REF: TypeId = TypeId(7);
    const SLICE_U8: TypeId = TypeId(8);
    const REF_SLICE_U8: TypeId = TypeId(9);
    const PTR_U8: TypeId = TypeId(10);
    const REF_U8: TypeId = TypeId(11);
    const PAIR: TypeId = TypeId(12);
    const I32: TypeId = TypeId(13);
    const PTR_I32: TypeId = TypeId(14);

    fn desc(id: TypeId, kind: TypeKind, size: u64) -> TypeDescriptor {
        let sized = size > 0 || !matches!(kind, TypeKind::Str | TypeKind::Slice(_));
        let zst = size == 0 && !matches!(kind, TypeKind::Str | TypeKind::Slice(_));
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                unsized_prefix: None,
                align: Some(size.clamp(1, 4)),
                sized,
                zst,
                one_zst: zst,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: false,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn int_desc(id: TypeId, signed: bool, bits: u32) -> TypeDescriptor {
        let mut d = desc(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
        d.js.int = Some(IntFacts {
            signed,
            bits,
            repr: IntRepr::Number,
        });
        d.js.byte_element = bits == 8;
        d
    }

    fn ptr_desc(id: TypeId, pointee: TypeId, form: PointerForm, raw: bool) -> TypeDescriptor {
        let kind = if raw {
            TypeKind::RawPointer {
                pointee,
                mutable: false,
            }
        } else {
            TypeKind::Reference {
                pointee,
                mutable: false,
            }
        };
        let mut d = desc(id, kind, 4);
        d.js.pointer_form = Some(form);
        d
    }

    fn test_interns() -> Interns {
        let mut interns = Interns::new();
        interns.intern_type(desc(UNIT, TypeKind::Tuple(Vec::new()), 0));
        interns.intern_type(desc(BOOL, TypeKind::Bool, 1));
        interns.intern_type(int_desc(U8, false, 8));
        interns.intern_type(int_desc(USIZE, false, 32));
        interns.intern_type(int_desc(ISIZE, true, 32));
        interns.intern_type(int_desc(I32, true, 32));
        interns.intern_type(desc(STR, TypeKind::Str, 0));
        interns.intern_type(ptr_desc(STR_REF, STR, PointerForm::Str, false));
        interns.intern_type(desc(SLICE_U8, TypeKind::Slice(U8), 0));
        interns.intern_type(ptr_desc(
            REF_SLICE_U8,
            SLICE_U8,
            PointerForm::FatSlice,
            false,
        ));
        interns.intern_type(ptr_desc(PTR_U8, U8, PointerForm::Slot, true));
        interns.intern_type(ptr_desc(REF_U8, U8, PointerForm::Slot, false));
        interns.intern_type(ptr_desc(PTR_I32, I32, PointerForm::Slot, true));
        interns.intern_type(desc(PAIR, TypeKind::Tuple(vec![PTR_U8, USIZE]), 8));
        interns
    }

    fn sig(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: arguments.len() as u16,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
            arguments,
            result,
        }
    }

    fn dummy_item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: n,
            },
            instantiation: n as u128,
        }
    }

    fn run_analyze(interns: &Interns, func: &Func) -> BTreeMap<ValueId, ByteView> {
        let plan = super::super::fuse::plan(func, None, &interns.types, interns, &Opts::default());
        byte_view_analysis::analyze(func, &plan, &interns.types, interns, None)
    }

    #[test]
    fn byte_view_slice_across_goto_loop_and_subslice() {
        let interns = test_interns();
        let mut b = FuncBuilder::new(dummy_item(1), sig(vec![STR_REF, USIZE], U8));
        let s = b.param(STR_REF, NO_ORIGIN);
        let idx = b.param(USIZE, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let bytes = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: s,
                from: STR_REF,
                to: REF_SLICE_U8,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let sub = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(bytes),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 1,
                        to: 0,
                        from_end: true,
                    },
                ],
                mutable: false,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let hdr = b.block();
        let lp = b.block_param(hdr, REF_SLICE_U8, NO_ORIGIN);
        b.terminate(e, RTerm::Goto(hdr, vec![sub]));
        let elem_ref = b.op(
            hdr,
            OpKind::BorrowView {
                root: Root::Value(lp),
                path: vec![Proj::Deref, Proj::Index(idx)],
                mutable: false,
            },
            REF_U8,
            NO_ORIGIN,
        );
        let ch = b.op(hdr, OpKind::Deref(elem_ref), U8, NO_ORIGIN);
        b.terminate(hdr, RTerm::Return(Some(ch)));
        let func = b.finish();

        let map = run_analyze(&interns, &func);
        assert_eq!(map[&bytes].mode, ByteViewMode::FullSlice);
        assert_eq!(map[&sub].mode, ByteViewMode::VarSlice);
        assert_eq!(map[&lp].mode, ByteViewMode::VarSlice);
        assert_eq!(map[&elem_ref].mode, ByteViewMode::ThinPtr);
        assert_eq!(map[&elem_ref].borrow_slice, Some(lp));
        assert_eq!(map[&bytes].base_str, Some(s));

        // Merging full-slice and subslice into the same block param promotes all to VarSlice.
        let mut bm = FuncBuilder::new(dummy_item(10), sig(vec![STR_REF, BOOL, USIZE], U8));
        let sm = bm.param(STR_REF, NO_ORIGIN);
        let flag = bm.param(BOOL, NO_ORIGIN);
        let idxm = bm.param(USIZE, NO_ORIGIN);
        let em = FuncBuilder::ENTRY;
        let fullm = bm.op(
            em,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: sm,
                from: STR_REF,
                to: REF_SLICE_U8,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let subm = bm.op(
            em,
            OpKind::BorrowView {
                root: Root::Value(fullm),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 1,
                        to: 0,
                        from_end: true,
                    },
                ],
                mutable: false,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let hdrm = bm.block();
        let lpm = bm.block_param(hdrm, REF_SLICE_U8, NO_ORIGIN);
        bm.terminate(
            em,
            RTerm::Switch {
                scrut: flag,
                scrut_ty: BOOL,
                cases: vec![(0, hdrm, vec![fullm])],
                default: (hdrm, vec![subm]),
            },
        );
        let eref = bm.op(
            hdrm,
            OpKind::BorrowView {
                root: Root::Value(lpm),
                path: vec![Proj::Deref, Proj::Index(idxm)],
                mutable: false,
            },
            REF_U8,
            NO_ORIGIN,
        );
        let chm = bm.op(hdrm, OpKind::Deref(eref), U8, NO_ORIGIN);
        bm.terminate(hdrm, RTerm::Return(Some(chm)));
        let mfunc = bm.finish();
        let mmap = run_analyze(&interns, &mfunc);
        assert_eq!(mmap[&fullm].mode, ByteViewMode::VarSlice);
        assert_eq!(mmap[&subm].mode, ByteViewMode::VarSlice);
        assert_eq!(mmap[&lpm].mode, ByteViewMode::VarSlice);
    }

    #[test]
    fn byte_view_raw_pointer_loop_compare_and_diff() {
        let mut interns = test_interns();
        let ptr_offset_from = interns.intern_str("ptr_offset_from");
        let mut b = FuncBuilder::new(dummy_item(2), sig(vec![STR_REF], ISIZE));
        let s = b.param(STR_REF, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let bytes = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v: s,
                from: STR_REF,
                to: REF_SLICE_U8,
            },
            REF_SLICE_U8,
            NO_ORIGIN,
        );
        let len = b.op(
            e,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: bytes,
                ty: REF_SLICE_U8,
            },
            USIZE,
            NO_ORIGIN,
        );
        let start = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: bytes,
                from: REF_SLICE_U8,
                to: PTR_U8,
            },
            PTR_U8,
            NO_ORIGIN,
        );
        let end = b.op(
            e,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: start,
                r: len,
                ty: PTR_U8,
            },
            PTR_U8,
            NO_ORIGIN,
        );
        let hdr = b.block();
        let cur = b.block_param(hdr, PTR_U8, NO_ORIGIN);
        b.terminate(e, RTerm::Goto(hdr, vec![start]));
        let cond = b.op(
            hdr,
            OpKind::Binary {
                op: BinaryOp::Lt,
                l: cur,
                r: end,
                ty: PTR_U8,
            },
            BOOL,
            NO_ORIGIN,
        );
        let step_blk = b.block();
        let exit_blk = b.block();
        b.terminate(
            hdr,
            RTerm::Switch {
                scrut: cond,
                scrut_ty: BOOL,
                cases: vec![(0, exit_blk, Vec::new())],
                default: (step_blk, Vec::new()),
            },
        );
        let _val = b.op(step_blk, OpKind::Deref(cur), U8, NO_ORIGIN);
        let one = b.op(
            step_blk,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            ISIZE,
            NO_ORIGIN,
        );
        let next = b.op(
            step_blk,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: cur,
                r: one,
                ty: PTR_U8,
            },
            PTR_U8,
            NO_ORIGIN,
        );
        b.terminate(step_blk, RTerm::Goto(hdr, vec![next]));
        let diff = b.op(
            exit_blk,
            OpKind::Intrinsic {
                name: ptr_offset_from,
                types: vec![U8],
                fallback: None,
                args: vec![cur, start],
                track_caller: None,
            },
            ISIZE,
            NO_ORIGIN,
        );
        b.terminate(exit_blk, RTerm::Return(Some(diff)));
        let func = b.finish();

        let map = run_analyze(&interns, &func);
        for v in [bytes, start, end, cur, next] {
            assert_eq!(map[&v].base_str, Some(s), "missing base_str for {v:?}");
        }
        assert_eq!(map[&bytes].mode, ByteViewMode::FullSlice);
        assert_eq!(map[&cur].mode, ByteViewMode::ThinPtr);
    }

    #[test]
    fn byte_view_fails_closed_on_escapes_and_mixed_bases() {
        let mut interns = test_interns();
        let cmp_bytes = interns.intern_str("compare_bytes");
        let unknown_fn = interns.intern_str("other_fn");

        // 1. Escape to call poisons the whole component.
        {
            let mut b = FuncBuilder::new(dummy_item(3), sig(vec![STR_REF], PTR_U8));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let bytes = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: REF_SLICE_U8,
                },
                REF_SLICE_U8,
                NO_ORIGIN,
            );
            let ptr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: bytes,
                    from: REF_SLICE_U8,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let _ = b.op(
                e,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: dummy_item(99),
                        link_name: unknown_fn,
                    },
                    function: None,
                    args: vec![bytes],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                UNIT,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(ptr)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }

        // 2. Expose provenance (`as usize`) poisons the whole component.
        {
            let mut b = FuncBuilder::new(dummy_item(4), sig(vec![STR_REF], USIZE));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let bytes = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: REF_SLICE_U8,
                },
                REF_SLICE_U8,
                NO_ORIGIN,
            );
            let ptr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: bytes,
                    from: REF_SLICE_U8,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let addr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PointerExposeProvenance,
                    v: ptr,
                    from: PTR_U8,
                    to: USIZE,
                },
                USIZE,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(addr)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }

        // 3. `compare_bytes` intrinsic poisons the component.
        {
            let mut b = FuncBuilder::new(dummy_item(5), sig(vec![STR_REF], I32));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let bytes = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: REF_SLICE_U8,
                },
                REF_SLICE_U8,
                NO_ORIGIN,
            );
            let ptr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: bytes,
                    from: REF_SLICE_U8,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let zero = b.op(
                e,
                OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
                USIZE,
                NO_ORIGIN,
            );
            let cmp = b.op(
                e,
                OpKind::Intrinsic {
                    name: cmp_bytes,
                    types: Vec::new(),
                    fallback: None,
                    args: vec![ptr, ptr, zero],
                    track_caller: None,
                },
                I32,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(cmp)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }

        // 4. Comparing thin pointers from different `base_str`s poisons both.
        {
            let mut b = FuncBuilder::new(dummy_item(6), sig(vec![STR_REF, STR_REF], BOOL));
            let s1 = b.param(STR_REF, NO_ORIGIN);
            let s2 = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let b1 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s1,
                    from: STR_REF,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let b2 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s2,
                    from: STR_REF,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let eq = b.op(
                e,
                OpKind::Binary {
                    op: BinaryOp::Eq,
                    l: b1,
                    r: b2,
                    ty: PTR_U8,
                },
                BOOL,
                NO_ORIGIN,
            );
            b.terminate(e, RTerm::Return(Some(eq)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }

        // 5. Casting `*const u8` -> `*const i32` and dereferencing poisons component.
        {
            let mut b = FuncBuilder::new(dummy_item(7), sig(vec![STR_REF], I32));
            let s = b.param(STR_REF, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let p_u8 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::Transmute,
                    v: s,
                    from: STR_REF,
                    to: PTR_U8,
                },
                PTR_U8,
                NO_ORIGIN,
            );
            let p_i32 = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: p_u8,
                    from: PTR_U8,
                    to: PTR_I32,
                },
                PTR_I32,
                NO_ORIGIN,
            );
            let val = b.op(e, OpKind::Deref(p_i32), I32, NO_ORIGIN);
            b.terminate(e, RTerm::Return(Some(val)));
            let func = b.finish();
            assert!(run_analyze(&interns, &func).is_empty());
        }
    }
}
