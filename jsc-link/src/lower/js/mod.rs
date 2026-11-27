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

pub mod ast_util;
pub(crate) mod byte_view;
pub mod call;
pub mod constant;
pub mod emit;
mod fn_opt;
pub mod function;
pub mod host;
pub mod i64_pair;
pub mod intrinsics;
pub mod loop_opt;
pub mod place;
pub mod post_opt;
pub mod ptr;
pub mod rvalue;
pub mod soa;
pub mod unsize;
pub mod value;

pub(crate) use byte_view as byte_view_analysis;
pub use byte_view::{ByteView, ByteViewMode};

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

/// Pending `RTerm::Return` payload for a callee returning
/// [`crate::plan::abi_flat::SentinelResultShape`].
pub(super) enum PendingSentinelReturn {
    Ok(Expr),
    Err(Option<Expr>),
    Virtual {
        val: Expr,
        err: Option<Expr>,
    },
    Canonical {
        value: Expr,
        shape: crate::plan::abi_flat::SentinelResultShape,
    },
}

/// The return payload parked by the last returning block for the `Region::Return` that follows.
pub(super) enum PendingReturn {
    /// Standard (or tagged/recursive flat-ABI) return expression.
    Value(Expr),
    /// Signed-`Int32` `(lo, hi)` pair for a `flat_abi` `i64_pair` return.
    I64Pair(Expr, Expr),
    /// Payload for a `flat_abi` `SentinelResult` return.
    Sentinel(PendingSentinelReturn),
}

/// How a virtual [`crate::plan::abi_flat::SentinelResultShape`] value is represented in [`JsCx`].
#[derive(Clone, Debug)]
pub(super) enum VirtualSentinelRep {
    /// Materialized in local variables `(val_var, err_var)` where `val_var === $se` signals
    /// `err_variant` and `err_var` holds the captured error payload (when non-ZST).
    Slot {
        val_var: String,
        err_var: Option<String>,
    },
    /// Inline `Move`/`Copy` alias forwarding lazily to `source`.
    InlineAlias(ValueId),
    /// Inline `Aggregate` or `Const(ConstTree::Variant)` emitted directly at `Return` or a
    /// virtual `Phi` edge copy.
    InlineConstruct,
}

#[derive(Clone, Debug)]
pub(super) struct VirtualSentinel {
    pub shape: crate::plan::abi_flat::SentinelResultShape,
    pub rep: VirtualSentinelRep,
}

type SentinelCandidateMap =
    BTreeMap<ValueId, (crate::plan::abi_flat::SentinelResultShape, SentinelCandKind)>;
type SentinelCandidateSet = (
    Option<crate::plan::abi_flat::SentinelResultShape>,
    SentinelCandidateMap,
);

#[derive(Clone, Copy, Eq, PartialEq)]
enum SentinelCandKind {
    Call,
    Phi,
    Construct,
    Alias(ValueId),
}

fn sentinel_variant_valid(
    shape: crate::plan::abi_flat::SentinelResultShape,
    variant: u32,
    field_count: usize,
) -> bool {
    (variant == shape.ok_variant
        && shape
            .ok_field
            .is_none_or(|(fi, _)| (fi as usize) < field_count))
        || (variant == shape.err_variant
            && shape
                .err_field
                .is_none_or(|(fi, _)| (fi as usize) < field_count))
}

/// Optional whole-program driver and analysis plans passed into [`JsCx::with_plans`].
///
/// Bundling these inputs into a single parameter struct eliminates the multi-call `.with_*()`
/// builder chain and ensures `heap` is already finalized before [`JsCx`] is constructed, so
/// `strict_i32_u32` and `nested_clone` are always computed against the final `HeapPlan`.
pub struct ProgramPlans<'a> {
    pub origins: Option<&'a OriginTable>,
    pub items: &'a host::ItemIndex,
    pub drop_glue: &'a host::DropGlueIndex,
    pub vtables: &'a unsize::VtableIndex,
    pub spills: &'a [JsName],
    pub abi: Option<&'a crate::plan::abi_flat::AbiPlan>,
    pub target_abi: Option<&'a crate::plan::abi_flat::FlatSig>,
    pub call_retargets: &'a crate::lower::CallRetargets,
    pub hoists_bigints: bool,
    pub byte_views: crate::analysis::byte_views::ByteViewPlan,
    pub heap: Option<&'a crate::analysis::heap::HeapPlan>,
    pub soa: Option<&'a crate::plan::soa_vec::SoaPlan>,
    pub nested_clone: crate::lower::NestedClone,
    pub ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'a>>,
}

impl Default for ProgramPlans<'_> {
    fn default() -> Self {
        Self {
            origins: None,
            items: host::EMPTY_ITEMS,
            drop_glue: host::EMPTY_DROP_GLUE,
            vtables: &unsize::EMPTY_VTABLES,
            spills: &[],
            abi: None,
            target_abi: None,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
            hoists_bigints: false,
            byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
            heap: None,
            soa: None,
            nested_clone: crate::lower::NestedClone::default(),
            ascii: None,
        }
    }
}

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
    /// Whole-program Structure-of-Arrays (SoA) plan for non-escaping `Vec<FlatStruct, Global>`.
    pub soa: Option<&'a crate::plan::soa_vec::SoaPlan>,
    /// Scalarized local SoA element references (`&T` / `&mut T`): maps SSA value to `(buf, off)`.
    pub(super) soa_refs: RefCell<BTreeMap<ValueId, (Expr, Expr)>>,
    /// Scalarized local SoA `bool` field references (`&bool` / `&mut bool`): `(buf, col, off)`.
    pub(super) soa_bool_refs: RefCell<BTreeMap<ValueId, (Expr, String, Expr)>>,
    /// Whether this function carries nested native Vec/String fields needing carrier-safe clone.
    pub nested_clone: crate::lower::NestedClone,
    /// Whole-program ASCII provenance facts for this function (`js-str=bytes`).
    ascii: Option<crate::analysis::ascii::FuncAsciiFacts<'a>>,
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
    /// Allocation-bearing expressions paired with the JIR value that produced them. The final
    /// JavaScript allocation walker uses these only for provenance; they never affect emission.
    allocation_origins: RefCell<Vec<(ValueId, Expr)>>,
    /// The hoisted `$tN` temporaries this body has asked for, in allocation order.
    temps: RefCell<Vec<String>>,
    /// The zombies this body carries.
    zombies: RefCell<Vec<FlatZombie>>,
    /// The origin lowering is currently at, for zombies and source map markers.
    origin: Cell<Option<OriginId>>,
    /// The payload the last returning block parked for the `Region::Return` that follows it.
    pending_return: RefCell<Option<PendingReturn>>,
    /// Virtual `SentinelResult` values kept unboxed across calls, phis, and returns (lazy).
    virtual_sentinels: std::cell::OnceCell<BTreeMap<ValueId, VirtualSentinel>>,
    /// Error-payload companion temps for `CopyOperand::Temp` edge-copy cycles on virtual sentinels.
    copy_temp_errs: RefCell<BTreeMap<u32, String>>,
    /// The defining op of every value an op defines: `(block, index)`.
    defs: BTreeMap<ValueId, (u32, u32)>,
    /// Overflow pairs that are never built, and how each is spelled
    /// (`rvalue::carried_overflow_pairs`).
    carried_pairs: BTreeMap<ValueId, rvalue::CarriedPair>,
    /// Unsigned 32-bit (`u32`/`usize`) SSA values whose uses all depend only on their low 32 bits
    /// (bitwise, shifts, 32-bit wrapping arithmetic, narrow casts), so they may stay in signed
    /// `Int32` without a trailing `>>> 0`.
    int32_only: std::collections::BTreeSet<ValueId>,
    /// Single-use `Cast(&T -> *const T)` values feeding a constant-1 `copy`/`copy_nonoverlapping`
    /// under Tier A without raw allocs; lowered without `__rt.box` (computed lazily on first use).
    direct_ref_copy_casts: std::cell::OnceCell<std::collections::BTreeSet<ValueId>>,
    /// Boxed scalar cells whose borrows only feed read-only `Deref` or `HostPayload::Map` key
    /// reads (`has`/`get`/`getmut`/`set`/`del`) under `ReleaseSpeed`, allowing the `[k]` box array
    /// to be elided completely.
    unboxed_map_keys: std::cell::OnceCell<(
        std::collections::BTreeSet<CellId>,
        BTreeMap<ValueId, CellId>,
    )>,
    /// `u32` SSA values read from `heap.smi_u32_fields` whose consumers all accept strictly
    /// normalized signed `Int32` (`-2^31..=2^31-1`), computed at construction from `plans.heap`.
    strict_i32_u32: std::collections::BTreeSet<ValueId>,
}

impl<'a> JsCx<'a> {
    /// Constructs a lowering context with all whole-program driver and analysis plans upfront.
    pub fn with_plans(
        func: &'a Func,
        plan: &'a Plan,
        table: &'a TypeTable,
        interns: &'a Interns,
        opts: &'a Opts,
        summary: &'a ItemSummary,
        plans: ProgramPlans<'a>,
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
        let effective_heap = plans
            .heap
            .unwrap_or(&crate::analysis::heap::DEFAULT_HEAP_PLAN);
        let mut nested_clone = plans.nested_clone;
        if effective_heap.native_string_enabled {
            nested_clone.native_string = true;
        }
        if effective_heap.native_vec_enabled {
            nested_clone.native_vec = true;
        }
        let strict_i32_u32 = rvalue::strict_i32_u32_values(
            func,
            table,
            interns,
            &defs,
            &carried_pairs,
            &effective_heap.smi_u32_fields,
        );
        JsCx {
            func,
            plan,
            table,
            interns,
            opts,
            summary,
            origins: plans.origins,
            items: plans.items,
            drop_glue: plans.drop_glue,
            vtables: plans.vtables,
            spills: plans.spills,
            abi: plans.abi,
            target_abi: plans.target_abi,
            call_retargets: plans.call_retargets,
            hoists_bigints: plans.hoists_bigints,
            byte_views: plans.byte_views,
            heap: plans.heap,
            soa: plans.soa,
            soa_refs: RefCell::new(BTreeMap::new()),
            soa_bool_refs: RefCell::new(BTreeMap::new()),
            nested_clone,
            ascii: plans.ascii,
            exprs: RefCell::new(BTreeMap::new()),
            pair_exprs: RefCell::new(BTreeMap::new()),
            carried_i64_overflow: RefCell::new(BTreeMap::new()),
            formal_param_names: RefCell::new(HashSet::new()),
            tagged_exprs: RefCell::new(BTreeMap::new()),
            byte_string_views: std::cell::OnceCell::new(),
            allocation_origins: RefCell::new(Vec::new()),
            temps: RefCell::new(Vec::new()),
            zombies: RefCell::new(Vec::new()),
            origin: Cell::new(None),
            pending_return: RefCell::new(None),
            virtual_sentinels: std::cell::OnceCell::new(),
            copy_temp_errs: RefCell::new(BTreeMap::new()),
            defs,
            carried_pairs,
            int32_only,
            direct_ref_copy_casts: std::cell::OnceCell::new(),
            unboxed_map_keys: std::cell::OnceCell::new(),
            strict_i32_u32,
        }
    }

    #[cfg(test)]
    pub fn new(
        func: &'a Func,
        plan: &'a Plan,
        table: &'a TypeTable,
        interns: &'a Interns,
        opts: &'a Opts,
        summary: &'a ItemSummary,
        origins: Option<&'a OriginTable>,
    ) -> Self {
        Self::with_plans(
            func,
            plan,
            table,
            interns,
            opts,
            summary,
            ProgramPlans {
                origins,
                ..ProgramPlans::default()
            },
        )
    }

    #[cfg(test)]
    pub fn with_soa(mut self, soa: Option<&'a crate::plan::soa_vec::SoaPlan>) -> Self {
        self.soa = soa;
        self
    }

    /// Whether this lowering holds a `str` as a byte string (`js-str=bytes`).
    pub(super) fn bstr_enabled(&self) -> bool {
        crate::lower::bstr::enabled_for(self.opts)
    }

    pub fn is_smi_u32_field(&self, struct_ty: TypeId, field: u32) -> bool {
        let fields = &self.heap().smi_u32_fields;
        !fields.is_empty() && fields.contains(&(self.table.peeled(struct_ty), field))
    }

    pub fn is_smi_u32_ptr(&self, ptr: ValueId) -> bool {
        let fields = &self.heap().smi_u32_fields;
        if fields.is_empty() {
            return false;
        }
        let Some(def) = self.def_of(ptr) else {
            return false;
        };
        let (OpKind::BorrowView { root, path, .. } | OpKind::RawView { root, path, .. }) =
            &def.kind
        else {
            return false;
        };
        crate::analysis::heap::view_final_struct_field(self.func, self.table, root, path)
            .is_some_and(|pair| fields.contains(&pair))
    }

    pub fn is_strict_i32_u32(&self, v: ValueId) -> bool {
        self.strict_i32_u32.contains(&v)
    }

    pub fn const_u32_bits(&self, v: ValueId) -> Option<u128> {
        let ty = self.ty_of(v).ok()?;
        if self.table.int_repr(ty) != rcgjs_object::jir::types::IntRepr::Number
            || self.table.int_info(ty) != Some((false, 32))
        {
            return None;
        }
        match &self.def_of(v)?.kind {
            OpKind::Const(rcgjs_object::jir::semantic::ConstTree::Scalar { bits, .. }) => {
                Some(*bits)
            }
            _ => None,
        }
    }

    pub fn smi_i32_store(expr: Expr) -> Expr {
        if let Expr::Num(n) = expr {
            if n.is_finite() && n.fract() == 0.0 && n >= 0.0 && n <= f64::from(u32::MAX) {
                return jsast::num(f64::from((n as u32) as i32));
            }
        }
        value::simplify_i32_wrap(expr)
    }

    fn map_key_unboxing(
        &self,
    ) -> &(
        std::collections::BTreeSet<CellId>,
        BTreeMap<ValueId, CellId>,
    ) {
        self.unboxed_map_keys.get_or_init(|| {
            use rcgjs_object::jir::repr::{Carrier, Proj, Root};
            use rcgjs_object::jir::semantic::{Callee, DropGlue, HostPayload};
            let mut cells_out = std::collections::BTreeSet::new();
            let mut views_out = BTreeMap::new();
            if self.opts.profile != Some(crate::opts::Profile::ReleaseSpeed)
                || self.func.cells.is_empty()
            {
                return (cells_out, views_out);
            }
            let mut store_counts = vec![0usize; self.func.cells.len()];
            let mut bad_cells = std::collections::BTreeSet::new();
            let mut cell_views: BTreeMap<CellId, Vec<ValueId>> = BTreeMap::new();
            let mut term_uses: std::collections::BTreeSet<ValueId> =
                std::collections::BTreeSet::new();
            let mut value_uses: BTreeMap<ValueId, Vec<(usize, usize)>> = BTreeMap::new();

            for (bi, block) in self.func.blocks.iter().enumerate() {
                for v in block.term.uses() {
                    term_uses.insert(v);
                }
                for (oi, op) in block.ops.iter().enumerate() {
                    for v in op.kind.uses() {
                        let list = value_uses.entry(v).or_default();
                        if list.last().copied() != Some((bi, oi)) {
                            list.push((bi, oi));
                        }
                    }
                    match &op.kind {
                        OpKind::CellStore { cell, .. } => {
                            if let Some(slot) = store_counts.get_mut(cell.0 as usize) {
                                *slot += 1;
                            }
                        }
                        OpKind::RawView {
                            root: Root::Cell(c),
                            ..
                        } => {
                            bad_cells.insert(*c);
                        }
                        OpKind::Drop {
                            root: Root::Cell(c),
                            glue,
                            ..
                        } if !matches!(glue, DropGlue::None) => {
                            bad_cells.insert(*c);
                        }
                        OpKind::BorrowView {
                            root: Root::Cell(c),
                            path,
                            mutable,
                        } => {
                            let Some(res) = op.result else {
                                bad_cells.insert(*c);
                                continue;
                            };
                            let res_def = self.func.values[res.0 as usize];
                            if !path.is_empty()
                                || *mutable
                                || res_def.carrier != Carrier::Baseline
                                || !value::is_slot(self.table, res_def.ty)
                                || self.plan.physical_slots.contains(&res)
                            {
                                bad_cells.insert(*c);
                            } else {
                                cell_views.entry(*c).or_default().push(res);
                            }
                        }
                        _ => {}
                    }
                }
            }

            let map_key_idx = |op: &Op| -> Option<usize> {
                let OpKind::Call { callee, host, .. } = &op.kind else {
                    return None;
                };
                let marker = match host {
                    Some(HostPayload::Map { marker }) => self.interns.str(*marker),
                    None => match callee {
                        Callee::Direct { link_name, .. } => self
                            .interns
                            .str(*link_name)
                            .and_then(|s| s.strip_prefix("rcgjs.map.")),
                        _ => None,
                    },
                    _ => None,
                }?;
                match marker {
                    "has" | "set" | "get" | "getmut" | "try_get" | "try_getmut" | "del" => Some(1),
                    _ => None,
                }
            };

            for (ci, cell_def) in self.func.cells.iter().enumerate() {
                let cell = CellId(ci as u32);
                if !cell_def.boxed
                    || bad_cells.contains(&cell)
                    || self.table.is_indirect(cell_def.ty)
                    || self.table.needs_clone(cell_def.ty)
                {
                    continue;
                }
                if self.plan.param_cells.contains_key(&cell) && store_counts[ci] > 1 {
                    continue;
                }
                let Some(roots) = cell_views.get(&cell) else {
                    continue;
                };
                let mut aliases = std::collections::BTreeSet::new();
                let mut queue = Vec::new();
                for &r in roots {
                    if aliases.insert(r) {
                        queue.push(r);
                    }
                }
                let mut saw_map_key = false;
                let mut ok = true;
                while let Some(ptr) = queue.pop() {
                    if term_uses.contains(&ptr) || self.plan.physical_slots.contains(&ptr) {
                        ok = false;
                        break;
                    }
                    let Some(uses) = value_uses.get(&ptr) else {
                        continue;
                    };
                    for &(bi, oi) in uses {
                        let use_op = &self.func.blocks[bi].ops[oi];
                        match &use_op.kind {
                            OpKind::Copy(src) | OpKind::Move(src) if *src == ptr => {
                                if let Some(res) = use_op.result {
                                    let res_def = self.func.values[res.0 as usize];
                                    if res_def.carrier != Carrier::Baseline
                                        || !value::is_slot(self.table, res_def.ty)
                                    {
                                        ok = false;
                                        break;
                                    }
                                    if aliases.insert(res) {
                                        queue.push(res);
                                    }
                                }
                            }
                            OpKind::BorrowView {
                                root: Root::Value(src),
                                path,
                                mutable: false,
                            } if *src == ptr && path.as_slice() == [Proj::Deref] => {
                                if let Some(res) = use_op.result {
                                    let res_def = self.func.values[res.0 as usize];
                                    if res_def.carrier != Carrier::Baseline
                                        || !value::is_slot(self.table, res_def.ty)
                                    {
                                        ok = false;
                                        break;
                                    }
                                    if aliases.insert(res) {
                                        queue.push(res);
                                    }
                                }
                            }
                            OpKind::Deref(p) if *p == ptr => {}
                            OpKind::Drop {
                                glue: DropGlue::None,
                                ..
                            } => {}
                            OpKind::Call { args, .. } => {
                                let Some(ki) = map_key_idx(use_op) else {
                                    ok = false;
                                    break;
                                };
                                let only_at_key =
                                    args.iter().enumerate().all(|(i, a)| *a != ptr || i == ki);
                                if !only_at_key {
                                    ok = false;
                                    break;
                                }
                                saw_map_key = true;
                            }
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if !ok {
                        break;
                    }
                }
                if ok && saw_map_key {
                    cells_out.insert(cell);
                    for v in aliases {
                        views_out.insert(v, cell);
                    }
                }
            }
            (cells_out, views_out)
        })
    }

    /// Returns the unboxed scalar cell backing `value` when `value` is a read-only `Map` key view.
    pub fn unboxed_map_key_cell(&self, value: ValueId) -> Option<CellId> {
        self.map_key_unboxing().1.get(&value).copied()
    }

    /// Whether `value` is a single-use `Cast(&T -> *const T)` whose sole use is as `src` of a
    /// constant-1 `copy`/`copy_nonoverlapping`, eligible to bypass `__rt.box` under Tier A without
    /// non-RC raw allocations.
    pub(super) fn is_direct_ref_copy_cast(&self, value: ValueId) -> bool {
        let heap = self.heap();
        if !heap.typed_heap_enabled
            || heap.non_rc_raw_alloc_reachable
            || self.byte_views.raw_byte_derefs
            || !self.byte_views.elide_scale_scalar
        {
            return false;
        }
        self.direct_ref_copy_casts
            .get_or_init(|| {
                crate::analysis::heap::direct_ref_copy_one_casts(
                    self.func,
                    self.table,
                    self.interns,
                )
            })
            .contains(&value)
    }

    /// Whether `value` is a 32-bit unsigned SSA value whose consumers all accept signed `Int32`.
    pub(super) fn is_int32_only(&self, value: ValueId) -> bool {
        self.int32_only.contains(&value)
    }

    /// Whether byte-slice references need the `__rt.byte_slice_ref` runtime barrier.
    ///
    /// Under Tier A (`typed_heap_enabled`) with no non-RC raw heap allocations and no wide-to-byte
    /// aliasing (`!raw_byte_derefs`), `_bv` is `null` and no fresh 1-byte raw `_blocks` entry can
    /// ever be retyped, making `byte_slice_ref(p)` an identity no-op.
    pub(super) fn needs_byte_slice_ref(&self) -> bool {
        let heap = self.heap();
        !(heap.typed_heap_enabled
            && !heap.non_rc_raw_alloc_reachable
            && !self.byte_views.raw_byte_derefs)
    }

    /// Whether every value in this function uses the baseline carrier (no `NativeVec`,
    /// `SliceView`, `HostString`, `StringBuilder`, or `NativeMap` carriers).
    pub(super) fn all_baseline_carriers(&self) -> bool {
        self.func
            .values
            .iter()
            .all(|v| v.carrier == rcgjs_object::jir::repr::Carrier::Baseline)
    }

    /// The same context, carrying the whole-program Tier A typed-heap and `_boxes` plan.
    #[cfg(test)]
    pub fn with_heap(mut self, heap: Option<&'a crate::analysis::heap::HeapPlan>) -> Self {
        self.heap = heap;
        if self.heap().native_string_enabled {
            self.nested_clone.native_string = true;
        }
        if self.heap().native_vec_enabled {
            self.nested_clone.native_vec = true;
        }
        self.strict_i32_u32 = rvalue::strict_i32_u32_values(
            self.func,
            self.table,
            self.interns,
            &self.defs,
            &self.carried_pairs,
            &self.heap().smi_u32_fields,
        );
        self
    }

    /// Returns the active whole-program [`crate::analysis::heap::HeapPlan`] (conservative default
    /// when `None`).
    pub fn heap(&self) -> &crate::analysis::heap::HeapPlan {
        self.heap
            .unwrap_or(&crate::analysis::heap::DEFAULT_HEAP_PLAN)
    }

    /// Whether this lowering hoists BigInt constants.
    pub fn hoists_bigints(&self) -> bool {
        self.hoists_bigints
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
        if !self.bstr_enabled() {
            return None;
        }
        self.byte_string_views
            .get_or_init(|| self.find_byte_string_views())
            .get(&v)
            .copied()
    }

    fn find_byte_string_views(&self) -> std::collections::BTreeMap<ValueId, ByteView> {
        let param_roots = self
            .target_abi
            .map_or_else(Vec::new, |sig| sig.byte_str_jir_positions());
        let abi = self.abi;
        byte_view_analysis::analyze_with(
            self.func,
            Some(self.plan),
            self.table,
            self.interns,
            &param_roots,
            false,
            &|op, arg_idx| {
                let OpKind::Call {
                    callee: rcgjs_object::jir::semantic::Callee::Direct { item, .. },
                    host: None,
                    ..
                } = &op.kind
                else {
                    return None;
                };
                let sig = abi?.call_signature(*item)?;
                sig.byte_str_jir_positions()
                    .into_iter()
                    .find_map(|(pos, mode)| (pos == arg_idx).then_some(mode))
            },
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
        if self.map_key_unboxing().0.contains(&cell) {
            return false;
        }
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

    /// Allocates a hoisted `$tN` temporary name declared in the function prelude.
    pub fn alloc_temp(&self) -> String {
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
    }

    /// Allocates a hoisted temporary holding `value` (`base.rs::temp`): the statement that fills
    /// it and the expression that reads it. Declared in the prelude.
    pub fn temp(&self, value: Expr) -> (Stmt, Expr) {
        let name = self.alloc_temp();
        (
            jsast::assign_stmt(jsast::id(name.clone()), value),
            jsast::id(name),
        )
    }

    /// Binds `expr` into a hoisted temporary in `stmts` unless it is already a bare identifier.
    pub fn bind_temp_if_needed(&self, expr: Expr, stmts: &mut Vec<Stmt>) -> Expr {
        if matches!(expr, Expr::Ident(_)) {
            expr
        } else {
            let (bind, id) = self.temp(expr);
            stmts.push(bind);
            id
        }
    }

    /// Binds `expr` into a hoisted temporary in `stmts` unless it is already a simple atomic
    /// expression (identifier or primitive literal) that is free and side-effect-free to re-read.
    pub fn bind_atom(&self, expr: Expr, stmts: &mut Vec<Stmt>) -> Expr {
        if ast_util::is_simple_atom(&expr) {
            expr
        } else {
            let (bind, id) = self.temp(expr);
            stmts.push(bind);
            id
        }
    }

    /// The private module-scope sentinel expression (`$sn`) for [`EnumRepr::SentinelNiche`].
    #[allow(
        dead_code,
        reason = "sentinel helper exposed for SentinelNiche lowering tests"
    )]
    pub fn sentinel_none_expr(&self) -> Option<Expr> {
        value::sentinel_none_expr(self.table)
    }

    /// The private module-scope sentinel expression (`$se`) for `SentinelResult` flat returns.
    pub fn sentinel_err_expr(&self) -> Option<Expr> {
        self.abi
            .and_then(|abi| abi.sentinel_err_name())
            .map(|name| jsast::id(name.as_str()))
    }

    /// Companion error-payload temp for `CopyOperand::Temp(index)` during parallel edge-copy cycles.
    pub(super) fn copy_temp_err_var(&self, temp_index: u32) -> String {
        if let Some(existing) = self.copy_temp_errs.borrow().get(&temp_index) {
            return existing.clone();
        }
        let name = self.alloc_temp();
        self.copy_temp_errs
            .borrow_mut()
            .insert(temp_index, name.clone());
        name
    }

    /// Returns the `SentinelResultShape` if `op` is a direct call to a `SentinelResult` callee.
    pub(super) fn sentinel_call_shape(
        &self,
        op: &Op,
    ) -> Option<crate::plan::abi_flat::SentinelResultShape> {
        let OpKind::Call {
            callee: rcgjs_object::jir::semantic::Callee::Direct { item, .. },
            host: None,
            ..
        } = &op.kind
        else {
            return None;
        };
        self.abi?.call_signature(*item)?.result.as_ref()?.sentinel()
    }

    /// Returns virtual `SentinelResult` metadata for `v`, if `v` is kept unboxed in this body.
    pub(super) fn virtual_sentinel(&self, v: ValueId) -> Option<&VirtualSentinel> {
        self.virtual_sentinels().get(&v)
    }

    /// Follows any `InlineAlias` chain to its backing `Slot` or `InlineConstruct` value.
    pub(super) fn resolve_virtual_sentinel(&self, mut v: ValueId) -> ValueId {
        let map = self.virtual_sentinels();
        for _ in 0..32 {
            match map.get(&v).map(|vs| &vs.rep) {
                Some(VirtualSentinelRep::InlineAlias(src)) => v = *src,
                _ => break,
            }
        }
        v
    }

    fn virtual_sentinels(&self) -> &BTreeMap<ValueId, VirtualSentinel> {
        self.virtual_sentinels.get_or_init(|| {
            if self.sentinel_err_expr().is_none() {
                return BTreeMap::new();
            }
            let Some((target_sentinel, mut cands)) = self.collect_sentinel_candidates() else {
                return BTreeMap::new();
            };
            self.propagate_sentinel_aliases(target_sentinel, &mut cands);
            self.build_virtual_sentinels(cands)
        })
    }

    fn collect_sentinel_candidates(&self) -> Option<SentinelCandidateSet> {
        use crate::plan::abi_flat::SentinelResultShape;
        use rcgjs_object::jir::repr::{Carrier, RBlockId};
        use rcgjs_object::jir::semantic::ConstTree;

        let target_sentinel = self
            .target_abi
            .and_then(|sig| sig.result.as_ref())
            .and_then(|result| result.sentinel());
        let mut call_shapes: BTreeMap<ValueId, SentinelResultShape> = BTreeMap::new();
        let mut relevant_shapes: BTreeMap<TypeId, SentinelResultShape> = BTreeMap::new();
        if let Some(shape) = target_sentinel {
            relevant_shapes.insert(shape.ty, shape);
        }
        for block in &self.func.blocks {
            for op in &block.ops {
                if let Some(shape) = self.sentinel_call_shape(op) {
                    relevant_shapes.insert(shape.ty, shape);
                    if let Some(res) = op.result {
                        call_shapes.insert(res, shape);
                    }
                }
            }
        }
        if relevant_shapes.is_empty() {
            return None;
        }

        let mut cands: BTreeMap<ValueId, (SentinelResultShape, SentinelCandKind)> = BTreeMap::new();
        let has_slot_or_inline = |v: ValueId| self.is_inline(v) || self.name_of(v).is_some();

        for (&res, &shape) in &call_shapes {
            let Some(def) = self.func.values.get(res.0 as usize) else {
                continue;
            };
            if def.ty == shape.ty && def.carrier == Carrier::Baseline && has_slot_or_inline(res) {
                cands.insert(res, (shape, SentinelCandKind::Call));
            }
        }
        for block in &self.func.blocks {
            if block.id == RBlockId(0) {
                continue;
            }
            for &param in &block.params {
                let Some(def) = self.func.values.get(param.0 as usize) else {
                    continue;
                };
                let Some(&shape) = relevant_shapes.get(&def.ty) else {
                    continue;
                };
                if def.carrier == Carrier::Baseline
                    && !self.plan.physical_slices.contains(&param)
                    && !self.plan.physical_slots.contains(&param)
                    && self.name_of(param).is_some()
                {
                    cands.insert(param, (shape, SentinelCandKind::Phi));
                }
            }
        }
        for block in &self.func.blocks {
            for op in &block.ops {
                let Some(res) = op.result else { continue };
                if cands.contains_key(&res) || !has_slot_or_inline(res) {
                    continue;
                }
                let Some(def) = self.func.values.get(res.0 as usize) else {
                    continue;
                };
                if def.carrier != Carrier::Baseline {
                    continue;
                }
                let Some(&shape) = relevant_shapes.get(&def.ty) else {
                    continue;
                };
                match &op.kind {
                    OpKind::Aggregate {
                        ty,
                        variant,
                        active_field: None,
                        fields,
                    } if *ty == shape.ty
                        && sentinel_variant_valid(shape, *variant, fields.len()) =>
                    {
                        cands.insert(res, (shape, SentinelCandKind::Construct));
                    }
                    OpKind::Const(ConstTree::Variant { variant, fields })
                        if sentinel_variant_valid(shape, *variant, fields.len()) =>
                    {
                        cands.insert(res, (shape, SentinelCandKind::Construct));
                    }
                    OpKind::Move(src) => {
                        cands.insert(res, (shape, SentinelCandKind::Alias(*src)));
                    }
                    OpKind::Copy(src) => {
                        let payload_needs_clone = shape
                            .ok_field
                            .is_some_and(|(_, t)| self.table.needs_clone(t))
                            || shape
                                .err_field
                                .is_some_and(|(_, t)| self.table.needs_clone(t));
                        if !payload_needs_clone {
                            cands.insert(res, (shape, SentinelCandKind::Alias(*src)));
                        }
                    }
                    _ => {}
                }
            }
        }

        Some((target_sentinel, cands))
    }

    fn resolves_to_inline_sentinel_construct(
        &self,
        mut v: ValueId,
        cands: &BTreeMap<ValueId, (crate::plan::abi_flat::SentinelResultShape, SentinelCandKind)>,
    ) -> bool {
        for _ in 0..32 {
            match cands.get(&v) {
                Some((_, SentinelCandKind::Construct)) => return self.is_inline(v),
                Some((_, SentinelCandKind::Alias(src))) if self.is_inline(v) => v = *src,
                _ => return false,
            }
        }
        true
    }

    fn sentinel_op_use_allowed(
        &self,
        op: &Op,
        u: ValueId,
        shape: crate::plan::abi_flat::SentinelResultShape,
        kind: SentinelCandKind,
        cands: &BTreeMap<ValueId, (crate::plan::abi_flat::SentinelResultShape, SentinelCandKind)>,
        construct_used: &mut std::collections::BTreeSet<ValueId>,
    ) -> bool {
        use rcgjs_object::jir::semantic::DropGlue;

        match &op.kind {
            OpKind::Discriminant(base) if *base == u => {
                !self.resolves_to_inline_sentinel_construct(u, cands)
            }
            OpKind::Field {
                base,
                variant,
                field,
            } if *base == u => {
                if self.resolves_to_inline_sentinel_construct(u, cands) {
                    return false;
                }
                let vi = variant.unwrap_or(0);
                let res_zst = op
                    .result
                    .and_then(|r| self.func.ty(r))
                    .is_some_and(|t| self.table.is_zst(t));
                (vi == shape.ok_variant
                    && (shape.ok_field.is_some_and(|(f, _)| f == *field) || res_zst))
                    || (vi == shape.err_variant
                        && (shape.err_field.is_some_and(|(f, _)| f == *field) || res_zst))
            }
            OpKind::Move(src) | OpKind::Copy(src) if *src == u => {
                let ok = op
                    .result
                    .is_some_and(|r| cands.get(&r).is_some_and(|(rs, _)| *rs == shape));
                if ok && kind == SentinelCandKind::Construct {
                    construct_used.insert(u);
                }
                ok
            }
            OpKind::Drop {
                root: Root::Value(r),
                path,
                glue: DropGlue::None,
            } if *r == u && path.is_empty() => true,
            _ => false,
        }
    }

    fn propagate_sentinel_aliases(
        &self,
        target_sentinel: Option<crate::plan::abi_flat::SentinelResultShape>,
        cands: &mut BTreeMap<
            ValueId,
            (crate::plan::abi_flat::SentinelResultShape, SentinelCandKind),
        >,
    ) {
        use rcgjs_object::jir::repr::{RBlockId, RTerm};
        use std::collections::BTreeSet;

        let block_index: BTreeMap<RBlockId, usize> = self
            .func
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id, i))
            .collect();
        let mut incoming: BTreeMap<RBlockId, Vec<&[ValueId]>> = BTreeMap::new();
        for block in &self.func.blocks {
            for (target, args) in block.term.edges() {
                incoming.entry(target).or_default().push(args);
            }
        }

        loop {
            let mut remove: BTreeSet<ValueId> = BTreeSet::new();

            // 1. Prune Phis whose incoming edges do not all supply matching candidates.
            for block in &self.func.blocks {
                if block.id == RBlockId(0) {
                    continue;
                }
                let edges = incoming.get(&block.id);
                for (pos, &param) in block.params.iter().enumerate() {
                    let Some(&(shape, SentinelCandKind::Phi)) = cands.get(&param) else {
                        continue;
                    };
                    let Some(edges) = edges.filter(|e| !e.is_empty()) else {
                        remove.insert(param);
                        continue;
                    };
                    let all_ok = edges.iter().all(|args| {
                        args.get(pos)
                            .and_then(|arg| cands.get(arg))
                            .is_some_and(|(s, _)| *s == shape)
                    });
                    if !all_ok {
                        remove.insert(param);
                    }
                }
            }

            // 2. Prune Aliases whose source is no longer a matching candidate.
            for (&v, &(shape, kind)) in cands.iter() {
                if let SentinelCandKind::Alias(src) = kind {
                    if !cands.get(&src).is_some_and(|(s, _)| *s == shape) {
                        remove.insert(v);
                    }
                }
            }

            // 3. Prune any candidate that has a disallowed use (and track Construct uses).
            let mut construct_used: BTreeSet<ValueId> = BTreeSet::new();
            for block in &self.func.blocks {
                for op in &block.ops {
                    for u in op.kind.uses() {
                        let Some(&(shape, kind)) = cands.get(&u) else {
                            continue;
                        };
                        if !self.sentinel_op_use_allowed(
                            op,
                            u,
                            shape,
                            kind,
                            cands,
                            &mut construct_used,
                        ) {
                            remove.insert(u);
                        }
                    }
                }
                match &block.term {
                    RTerm::Return(Some(u)) => {
                        if let Some(&(shape, kind)) = cands.get(u) {
                            if target_sentinel != Some(shape) {
                                remove.insert(*u);
                            } else if kind == SentinelCandKind::Construct {
                                construct_used.insert(*u);
                            }
                        }
                    }
                    RTerm::Switch { scrut, .. } if cands.contains_key(scrut) => {
                        remove.insert(*scrut);
                    }
                    RTerm::Throw(Some(u)) if cands.contains_key(u) => {
                        remove.insert(*u);
                    }
                    _ => {}
                }
                for (target, args) in block.term.edges() {
                    let target_params = block_index
                        .get(&target)
                        .map(|&bi| self.func.blocks[bi].params.as_slice());
                    for (pos, arg) in args.iter().enumerate() {
                        let Some(&(shape, kind)) = cands.get(arg) else {
                            continue;
                        };
                        let ok = target != RBlockId(0)
                            && target_params
                                .and_then(|ps| ps.get(pos))
                                .and_then(|p| cands.get(p))
                                .is_some_and(|(ps, _)| *ps == shape);
                        if !ok {
                            remove.insert(*arg);
                        } else if kind == SentinelCandKind::Construct {
                            construct_used.insert(*arg);
                        }
                    }
                }
            }

            // 4. Do not virtualize Construct values that never flow to a virtual Alias/Phi/Return.
            for (&v, &(_, kind)) in cands.iter() {
                if kind == SentinelCandKind::Construct && !construct_used.contains(&v) {
                    remove.insert(v);
                }
            }

            if remove.is_empty() {
                break;
            }
            for v in remove {
                cands.remove(&v);
            }
        }
    }

    fn build_virtual_sentinels(
        &self,
        cands: BTreeMap<ValueId, (crate::plan::abi_flat::SentinelResultShape, SentinelCandKind)>,
    ) -> BTreeMap<ValueId, VirtualSentinel> {
        let mut out = BTreeMap::new();
        for (v, (shape, kind)) in cands {
            let slot_rep = || {
                let val_var = self.name_of(v).expect("checked name").to_owned();
                let err_var = shape.err_field.map(|_| self.alloc_temp());
                VirtualSentinelRep::Slot { val_var, err_var }
            };
            let rep = match kind {
                SentinelCandKind::Call => {
                    let val_var = if self.is_inline(v) {
                        self.alloc_temp()
                    } else {
                        self.name_of(v).expect("checked name").to_owned()
                    };
                    let err_var = shape.err_field.map(|_| self.alloc_temp());
                    VirtualSentinelRep::Slot { val_var, err_var }
                }
                SentinelCandKind::Phi => slot_rep(),
                SentinelCandKind::Construct => {
                    if self.is_inline(v) {
                        VirtualSentinelRep::InlineConstruct
                    } else {
                        slot_rep()
                    }
                }
                SentinelCandKind::Alias(src) => {
                    if self.is_inline(v) {
                        VirtualSentinelRep::InlineAlias(src)
                    } else {
                        slot_rep()
                    }
                }
            };
            out.insert(v, VirtualSentinel { shape, rep });
        }
        out
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
