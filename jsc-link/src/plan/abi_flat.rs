//! The flat direct-call ABI: an aggregate argument passed as its fields.
//!
//! The baseline calling convention gives a Rust argument one JavaScript argument, so
//! `X64::add(&mut h1, h2)` hands the callee two objects — and the callee, which does nothing but
//! read and write `high` and `low`, has to keep them objects because the caller built them that
//! way. Nothing local can fix that: the shape of a call is a fact about *both* ends, so it is a
//! plan over the whole program rather than a pass over one function.
//!
//! [`plan`] picks the callees whose calling convention this program may change and rewrites both
//! ends of them:
//!
//! * the **callee** takes `k` scalar parameters instead of the aggregate and rebuilds the aggregate
//!   in its entry block, which the partial escape analysis then takes apart again;
//! * the **caller** passes `Field(a, i)` for each field instead of `a`;
//! * a field-only aggregate **result** returns its first field normally and its remaining fields
//!   through the same spill registers used for mutable-reference copy-back, then the caller
//!   rebuilds the semantic result for partial escape analysis to remove.
//!
//! A `&mut T` parameter is copied in the same way and copied *back* at every `Return`: the first
//! out-field rides the callee's return value when the callee returned `()`, and the rest go
//! through **spill registers** — module-scope `let $r1; let $r2; ...` bindings the link step
//! synthesizes as items of their own ([`AbiPlan::spill_items`]), that the callee writes
//! ([`OpKind::SpillWrite`]) right before it returns and the caller reads
//! ([`OpKind::SpillRead`]) right after the call. Flat aggregate results use the same protocol:
//! lane zero is the ordinary return, and lanes one onward precede copy-back fields in the spill
//! bindings.
//!
//! # What makes a callee eligible
//!
//! The rewrite changes what the item's JavaScript declaration accepts. An internal item may change
//! in place only when every reference is a direct call the plan also rewrites. An exported item or
//! one named by a function constant, static, vtable, or function-pointer coercion instead keeps its
//! canonical body and may receive one private flat clone; only proven direct callers retarget to
//! that clone. This is proved, not assumed: candidates still need a plain parameter list (no spread
//! tuple or `#[track_caller]`) and a body the lane lowered. Intrinsic fallbacks, host-marker
//! helpers, drop glue whose names are resolved by type, tracked callers, and other unmodeled
//! boundaries remain refusals. A program split into chunks refuses the plan whole: a spill register
//! is one module-scope binding and may not cross a chunk boundary.
//!
//! Which *parameters* go flat is [`crate::analysis::param_use`]'s question, closed over a fixed
//! point here: a parameter forwarded to another callee's parameter is field-only exactly when that
//! parameter is itself flat.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::analysis::param_use::{self, ParamKind, ParamUse};
use crate::analysis::refs::{self, RefScan};
use crate::item::{ItemKind, ItemMap, JsName, LinkItem, Linkage};
use crate::jir::decline::{BoundaryDecline, Decline, LowerDecline, OpClass};
use crate::jir::origin::OriginId;
use crate::jir::repr::{Carrier, Func, Op, OpKind, Proj, RTerm, Root, ValueDef, ValueId};
use crate::jir::semantic::{BodySemantic, Callee, ConstTree, ItemId, TypeId};
use crate::jir::summary::{self, ItemClass};
use crate::jir::types::{
    EnumRepr, FieldKey, NominalAbi, NominalForm, PinnedBoundary, PointerForm, TypeKind, TypeTable,
};
use crate::lane_report::push_string;
use crate::lower::js::emit::spill_name;
use crate::opt::candidates;
use crate::opts::Opts;

/// One parameter the plan flattens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlatParam {
    /// The aggregate type the parameter carries, or that a reference parameter points at.
    pub ty: TypeId,
    /// The field types, in **declaration** order — the shape the callee rebuilds.
    pub field_tys: Vec<TypeId>,
    /// The fields that become JavaScript parameters, by index into `field_tys`.
    ///
    /// A zero-sized field is `undefined` wherever it appears, so passing one says nothing: it is
    /// left out here and rebuilt in the callee as its own constant. `Context` — whose `ext` and
    /// two marker fields are zero-sized — is what makes the difference visible.
    pub passed: Vec<u32>,
    /// Whether the Rust parameter is a reference to the aggregate rather than the aggregate.
    pub by_ref: bool,
    /// Whether that reference is declared mutable.
    pub mutable: bool,
    /// Whether the callee ever writes through it, and so whether the fields have to be copied
    /// back at every `Return`. A `&mut T` the callee only reads is a `&T` as far as the ABI goes.
    pub written: bool,
    /// Whether these are the physical `buf`, `off`, and `len` lanes of a slice rather than
    /// declaration-order aggregate fields.
    pub slice: bool,
    /// Whether this 64-bit integer (`i64`/`u64`) travels as two signed-`Int32` `(lo, hi)` lanes.
    pub i64_pair: bool,
    /// A two-variant enum carried as its physical tag followed by the payload properties named
    /// here. Unlike fixed products, this shape stays semantic JIR until JavaScript lowering: the
    /// lowerer can project and rebuild the ordinary enum object without inventing a second enum
    /// IR or guessing a dynamic `Aggregate` variant.
    pub tagged: Option<TaggedShape>,
    /// A nested fixed product carried as recursively flattened scalar/pointer lanes. Like tagged
    /// values and slices, this remains one semantic JIR value and expands only at JavaScript
    /// lowering, so nested aggregates do not require a second semantic IR.
    pub recursive: Option<RecursiveShape>,
}

impl FlatParam {
    /// How many JavaScript parameters the flat parameter takes.
    pub fn fields(&self) -> u32 {
        if self.i64_pair {
            return 2;
        }
        self.tagged.as_ref().map_or_else(
            || {
                self.recursive
                    .as_ref()
                    .map_or(self.passed.len() as u32, RecursiveShape::fields)
            },
            TaggedShape::fields,
        )
    }
}

/// One physical component of a semantic leaf value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaneComponent {
    /// The JavaScript value itself.
    Value,
    /// The backing buffer of a slot or fat slice pointer.
    Buffer,
    /// The element offset of a slot or fat slice pointer.
    Offset,
    /// The element length of a fat slice pointer.
    Length,
}

/// A recursively physical fixed product.
///
/// `Product` preserves the source aggregate's declaration-order fields, while `Leaf` describes
/// the one-to-three JavaScript lanes that carry an indivisible semantic leaf. Zero-sized fields
/// remain in the tree so canonical reconstruction retains the source shape without spending an
/// ABI lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductNode {
    Zst {
        ty: TypeId,
    },
    Leaf {
        ty: TypeId,
        components: Vec<LaneComponent>,
    },
    Product {
        ty: TypeId,
        fields: Vec<ProductNode>,
    },
}

impl ProductNode {
    pub(crate) fn fields(&self) -> u32 {
        match self {
            ProductNode::Zst { .. } => 0,
            ProductNode::Leaf { components, .. } => components.len() as u32,
            ProductNode::Product { fields, .. } => fields.iter().map(ProductNode::fields).sum(),
        }
    }

    pub(crate) fn lane_types(&self, out: &mut Vec<TypeId>) {
        match self {
            ProductNode::Zst { .. } => {}
            ProductNode::Leaf { ty, components } => {
                out.extend(std::iter::repeat_n(*ty, components.len()));
            }
            ProductNode::Product { fields, .. } => {
                for field in fields {
                    field.lane_types(out);
                }
            }
        }
    }

    pub(crate) fn lane_components(&self, out: &mut Vec<(TypeId, LaneComponent)>) {
        match self {
            ProductNode::Zst { .. } => {}
            ProductNode::Leaf { ty, components } => {
                out.extend(components.iter().map(|component| (*ty, *component)));
            }
            ProductNode::Product { fields, .. } => {
                for field in fields {
                    field.lane_components(out);
                }
            }
        }
    }
}

/// A fixed product whose nested fields cross a private direct edge as physical leaves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecursiveShape {
    pub ty: TypeId,
    pub root: ProductNode,
    /// Top-level semantic fields accepted by the result-use proof.
    pub fields: Vec<u32>,
}

impl RecursiveShape {
    pub fn fields(&self) -> u32 {
        self.root.fields()
    }

    pub(crate) fn lane_types(&self) -> Vec<TypeId> {
        let mut out = Vec::new();
        self.root.lane_types(&mut out);
        out
    }
}

/// One payload property of a tagged physical product.
///
/// `variant` and `field` name a representative declaration whose JavaScript key carries the
/// lane. When both variants use the same key and type (as `Result<T, T>` does), they share one
/// lane. Missing properties are `undefined` and are never read on the inactive variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaggedLane {
    pub variant: u32,
    pub field: u32,
    /// Variants carrying the same canonical property with this exact product shape.
    pub present_variants: Vec<u32>,
    pub node: ProductNode,
}

/// The common product description for a small tagged value: `tag, payload...`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaggedShape {
    pub ty: TypeId,
    pub lanes: Vec<TaggedLane>,
    /// Every semantic payload projection accepted by the shape, including zero-sized fields and
    /// fields that share another variant's physical property lane.
    pub fields: Vec<(u32, u32)>,
}

impl TaggedShape {
    /// The JavaScript arity: one tag plus the live payload properties.
    pub fn fields(&self) -> u32 {
        1 + self
            .lanes
            .iter()
            .map(|lane| lane.node.fields())
            .sum::<u32>()
    }

    pub(crate) fn lane_types(&self) -> Vec<TypeId> {
        let mut out = Vec::new();
        for lane in &self.lanes {
            lane.node.lane_types(&mut out);
        }
        out
    }
}

/// One aggregate result the plan returns as independent fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlatResult {
    /// The aggregate type the baseline signature returns.
    pub ty: TypeId,
    /// The fields in declaration order, which is also the caller's rebuild order.
    pub field_tys: Vec<TypeId>,
    /// The non-zero-sized fields that travel as result lanes, by index into `field_tys`.
    pub passed: Vec<u32>,
    /// Whether this 64-bit integer (`i64`/`u64`) returns as `(lo, hi)` (`lo` primary, `hi` in
    /// spill register 1).
    pub i64_pair: bool,
    /// The tagged-product form, when this is a small two-variant enum rather than a fixed
    /// aggregate. The physical primary return is the tag; payloads use the ordinary spill lanes.
    pub tagged: Option<TaggedShape>,
    /// The recursive fixed-product form, when nested fields remain one semantic JIR result and
    /// expand only in the JavaScript ABI.
    pub recursive: Option<RecursiveShape>,
}

impl FlatResult {
    /// How many physical result lanes this semantic result takes.
    fn fields(&self) -> u32 {
        if self.i64_pair {
            return 2;
        }
        self.tagged.as_ref().map_or_else(
            || {
                self.recursive
                    .as_ref()
                    .map_or(self.passed.len() as u32, RecursiveShape::fields)
            },
            TaggedShape::fields,
        )
    }
}

/// One callee's flat signature: what happens to each parameter, and how many spill registers its
/// out-fields need.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FlatSig {
    /// One entry per JavaScript parameter of the baseline signature; `None` is unchanged.
    pub params: Vec<Option<FlatParam>>,
    /// The aggregate result returned as lanes, or `None` when the result stays canonical.
    pub result: Option<FlatResult>,
    /// How many spill registers the callee writes. Registers are numbered from 1.
    pub spill: u32,
    /// Whether the callee's `()` return carries the first out-field instead.
    pub returns_out_field: bool,
}

/// One aggregate-bearing item the physical ABI planner selected or left canonical.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbiDecision {
    pub item: JsName,
    pub shape: Option<String>,
    pub selected: bool,
    pub reason: Option<String>,
    pub cross_crate: CrossCrateEvidence,
}

/// Summary and caller evidence consumed by one physical ABI decision.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CrossCrateEvidence {
    pub summary_available: bool,
    pub summary_validated: bool,
    pub body_decoded: bool,
    pub caller_count: u32,
    pub crosses_crate: bool,
    pub producer_crate: Option<String>,
    pub caller_crates: Vec<String>,
}

impl FlatSig {
    /// Positions of semantic slice parameters before aggregate parameters expand in JIR-R.
    /// Analyses that run before [`AbiPlan::rewrite`] use these positions to recognize a call
    /// whose slice object will be consumed as `buf, off, len` lanes.
    pub(crate) fn slice_param_positions(&self) -> Vec<usize> {
        self.params
            .iter()
            .enumerate()
            .filter_map(|(position, param)| {
                param
                    .as_ref()
                    .is_some_and(|param| param.slice)
                    .then_some(position)
            })
            .collect()
    }

    /// Positions of slice values after aggregate parameters have already expanded in JIR-R.
    /// Phase C uses these positions to expand only the remaining semantic slice value.
    pub(crate) fn slice_jir_positions(&self) -> Vec<usize> {
        let mut positions = Vec::new();
        let mut cursor = 0;
        for param in &self.params {
            match param {
                Some(param) if param.slice => {
                    positions.push(cursor);
                    cursor += 1;
                }
                Some(param)
                    if param.i64_pair || param.tagged.is_some() || param.recursive.is_some() =>
                {
                    cursor += 1
                }
                Some(param) => cursor += param.fields() as usize,
                None => cursor += 1,
            }
        }
        positions
    }

    /// Tagged values keep one semantic JIR parameter, just as slices do, while phase C expands
    /// that position to `tag, payload...` in the JavaScript declaration and direct calls.
    pub(crate) fn tagged_jir_positions(&self) -> Vec<(usize, TaggedShape)> {
        let mut positions = Vec::new();
        let mut cursor = 0;
        for param in &self.params {
            match param {
                Some(param) if param.tagged.is_some() => {
                    positions.push((
                        cursor,
                        param.tagged.clone().expect("checked tagged parameter"),
                    ));
                    cursor += 1;
                }
                Some(param) if param.slice || param.i64_pair || param.recursive.is_some() => {
                    cursor += 1
                }
                Some(param) => cursor += param.fields() as usize,
                None => cursor += 1,
            }
        }
        positions
    }

    /// Recursive products also remain one semantic JIR value and expand only in JavaScript.
    pub(crate) fn recursive_jir_positions(&self) -> Vec<(usize, RecursiveShape)> {
        let mut positions = Vec::new();
        let mut cursor = 0;
        for param in &self.params {
            match param {
                Some(param) if param.recursive.is_some() => {
                    positions.push((
                        cursor,
                        param
                            .recursive
                            .clone()
                            .expect("checked recursive parameter"),
                    ));
                    cursor += 1;
                }
                Some(param) if param.slice || param.i64_pair || param.tagged.is_some() => {
                    cursor += 1
                }
                Some(param) => cursor += param.fields() as usize,
                None => cursor += 1,
            }
        }
        positions
    }

    /// Positions of 64-bit integer `(lo, hi)` pair parameters in JIR parameter numbering.
    pub(crate) fn i64_pair_jir_positions(&self) -> Vec<usize> {
        let mut positions = Vec::new();
        let mut cursor = 0;
        for param in &self.params {
            match param {
                Some(param) if param.i64_pair => {
                    positions.push(cursor);
                    cursor += 1;
                }
                Some(param)
                    if param.slice || param.tagged.is_some() || param.recursive.is_some() =>
                {
                    cursor += 1
                }
                Some(param) => cursor += param.fields() as usize,
                None => cursor += 1,
            }
        }
        positions
    }

    /// The out-fields the callee copies back, as `(parameter position, field index)`, in the order
    /// both ends walk them.
    fn out_fields(&self) -> Vec<(usize, u32)> {
        let mut out = Vec::new();
        for (position, param) in self.params.iter().enumerate() {
            let Some(param) = param else { continue };
            if param.by_ref && param.mutable && param.written {
                out.extend(param.passed.iter().map(|field| (position, *field)));
            }
        }
        out
    }

    /// The type of out-field `index`.
    fn out_ty(&self, out: (usize, u32)) -> TypeId {
        self.params[out.0]
            .as_ref()
            .map(|param| param.field_tys[out.1 as usize])
            .expect("an out-field names a flat parameter")
    }

    /// Result lanes beyond the primary JavaScript return.
    fn result_spills(&self) -> u32 {
        self.result.as_ref().map_or(0, |result| result.fields() - 1)
    }
}

/// The whole plan: the callees whose ABI changed, the ones considered and refused, and how many
/// spill registers the program needs.
#[derive(Clone, Debug, Default)]
pub struct AbiPlan {
    flat: BTreeMap<ItemId, FlatSig>,
    /// The JavaScript name of every item in `flat`, for the sidecar.
    names: BTreeMap<ItemId, JsName>,
    declined: BTreeMap<JsName, Decline>,
    decisions: BTreeMap<JsName, AbiDecision>,
    /// Every item whose `Func` the plan rewrites: the flat callees and all of their callers.
    touched: BTreeSet<JsName>,
    registers: u32,
    /// The name of each register, register 1 first. Allocated against the program's own names
    /// (see [`allocate_spill_names`]), so it is `$rN` only when nothing else is called that.
    spill_names: Vec<JsName>,
    /// Private flat bodies that retain a canonical exported or indirect entry point.
    clones: BTreeMap<JsName, JsName>,
    /// Direct callers retargeted from a canonical body to its private flat clone.
    call_retargets: BTreeMap<JsName, crate::lower::CallRetargets>,
    /// Proven direct callers of each selected semantic item, used to keep exact-cost groups
    /// coupled where two ABI changes meet in one body.
    callers: BTreeMap<ItemId, BTreeSet<JsName>>,
}

impl AbiPlan {
    /// Whether the plan makes the program no promises, which is what a retry falls back to.
    pub fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }

    /// The plan a retry falls back to: no callee flattened, nothing touched, but the refusals
    /// still recorded, so the sidecar says what was considered even on a run that kept none of it.
    pub fn emptied(&self) -> AbiPlan {
        let decisions = self
            .decisions
            .iter()
            .map(|(name, decision)| {
                let mut decision = decision.clone();
                if decision.selected {
                    decision.selected = false;
                    decision.reason = Some("validation.plan-retry".to_owned());
                }
                (name.clone(), decision)
            })
            .collect();
        AbiPlan {
            declined: self.declined.clone(),
            decisions,
            ..AbiPlan::default()
        }
    }

    /// Adds the complete physical label vector and private ABI choices to an exact-cost key.
    pub(crate) fn extend_cost_key(&self, out: &mut Vec<u8>) {
        for (item, sig) in &self.flat {
            out.extend_from_slice(format!("|flat:{item:?}:{sig:?}").as_bytes());
        }
        for (clone, source) in &self.clones {
            out.extend_from_slice(format!("|clone:{clone}:{source}").as_bytes());
        }
        for spill in &self.spill_names {
            out.extend_from_slice(format!("|spill:{spill}").as_bytes());
        }
        for (caller, retargets) in &self.call_retargets {
            out.extend_from_slice(format!("|retarget:{caller}:{retargets:?}").as_bytes());
        }
    }

    /// ABI-connected groups for exact economics.
    ///
    /// Two signatures stay together when one calls the other or when they share a caller. The
    /// latter matters because result lanes and copy-back lanes share spill bindings in that body;
    /// pricing either side alone would omit their interaction.
    pub(crate) fn economic_groups(&self) -> Vec<BTreeSet<ItemId>> {
        let mut groups: Vec<BTreeSet<ItemId>> = self
            .flat
            .keys()
            .map(|item| BTreeSet::from([*item]))
            .collect();
        let mut left = 0;
        while left < groups.len() {
            let mut right = left + 1;
            while right < groups.len() {
                let coupled = groups[left]
                    .iter()
                    .any(|a| groups[right].iter().any(|b| self.items_are_coupled(*a, *b)));
                if coupled {
                    let merged = groups.remove(right);
                    groups[left].extend(merged);
                    // The newly merged members may connect this component to a group we already
                    // compared. Restart the row so ordering cannot split one ABI-connected
                    // component (A--C--B with the groups initially ordered A,B,C).
                    right = left + 1;
                } else {
                    right += 1;
                }
            }
            left += 1;
        }
        groups
    }

    /// The executable part of this plan restricted to `selected` semantic items.
    pub(crate) fn subset(&self, selected: &BTreeSet<ItemId>) -> AbiPlan {
        let flat: BTreeMap<ItemId, FlatSig> = self
            .flat
            .iter()
            .filter(|(item, _)| selected.contains(item))
            .map(|(item, sig)| (*item, sig.clone()))
            .collect();
        let names: BTreeMap<ItemId, JsName> = self
            .names
            .iter()
            .filter(|(item, _)| selected.contains(item))
            .map(|(item, name)| (*item, name.clone()))
            .collect();
        let targets: BTreeSet<JsName> = names.values().cloned().collect();
        let clones = self
            .clones
            .iter()
            .filter(|(clone, _)| targets.contains(*clone))
            .map(|(clone, source)| (clone.clone(), source.clone()))
            .collect();
        let call_retargets = self
            .call_retargets
            .iter()
            .filter_map(|(caller, retargets)| {
                let retained: crate::lower::CallRetargets = retargets
                    .iter()
                    .filter(|(item, _)| selected.contains(item))
                    .map(|(item, target)| (*item, target.clone()))
                    .collect();
                (!retained.is_empty()).then(|| (caller.clone(), retained))
            })
            .collect();
        let callers: BTreeMap<ItemId, BTreeSet<JsName>> = self
            .callers
            .iter()
            .filter(|(item, _)| selected.contains(item))
            .map(|(item, callers)| (*item, callers.clone()))
            .collect();
        let mut touched = targets;
        touched.extend(callers.values().flatten().cloned());
        let registers = flat.values().map(|sig| sig.spill).max().unwrap_or(0);
        let spill_names = self
            .spill_names
            .iter()
            .take(registers as usize)
            .cloned()
            .collect();
        AbiPlan {
            flat,
            names,
            declined: self.declined.clone(),
            decisions: self.decisions.clone(),
            touched,
            registers,
            spill_names,
            clones,
            call_retargets,
            callers,
        }
    }

    /// Applies the accepted economic groups and records one terminal reason for every rejected
    /// legal signature while retaining all summary and boundary evidence.
    pub(crate) fn with_economic_selection(
        &self,
        selected: &BTreeSet<ItemId>,
        reasons: &BTreeMap<ItemId, String>,
    ) -> AbiPlan {
        let mut plan = self.subset(selected);
        let by_source: BTreeMap<JsName, ItemId> = self
            .flat
            .keys()
            .filter_map(|item| Some((self.source_name(*item)?.clone(), *item)))
            .collect();
        plan.decisions = self
            .decisions
            .iter()
            .map(|(name, decision)| {
                let mut decision = decision.clone();
                if decision.selected {
                    let item = by_source
                        .get(name)
                        .expect("every selected decision has a flat signature");
                    decision.selected = selected.contains(item);
                    decision.reason = (!decision.selected).then(|| {
                        terminal_reason(
                            reasons
                                .get(item)
                                .map(String::as_str)
                                .unwrap_or("validation.physical-measurement"),
                        )
                    });
                }
                (name.clone(), decision)
            })
            .collect();
        plan
    }

    pub(crate) fn source_name(&self, item: ItemId) -> Option<&JsName> {
        let target = self.names.get(&item)?;
        Some(self.clones.get(target).unwrap_or(target))
    }

    fn items_are_coupled(&self, a: ItemId, b: ItemId) -> bool {
        let a_callers = self.callers.get(&a);
        let b_callers = self.callers.get(&b);
        let share_caller = a_callers
            .zip(b_callers)
            .is_some_and(|(a, b)| a.iter().any(|caller| b.contains(caller)));
        let a_source = self.source_name(a);
        let b_source = self.source_name(b);
        share_caller
            || a_source
                .is_some_and(|source| b_callers.is_some_and(|callers| callers.contains(source)))
            || b_source
                .is_some_and(|source| a_callers.is_some_and(|callers| callers.contains(source)))
    }

    /// Aggregate-bearing items considered by the physical ABI planner.
    pub(crate) fn decisions(&self) -> impl Iterator<Item = &AbiDecision> {
        self.decisions.values()
    }

    fn consider(&mut self, name: &JsName, reason: impl Into<String>) {
        self.decisions.entry(name.clone()).or_insert(AbiDecision {
            item: name.clone(),
            shape: None,
            selected: false,
            reason: Some(reason.into()),
            cross_crate: CrossCrateEvidence::default(),
        });
    }

    fn record_evidence(&mut self, name: &JsName, evidence: CrossCrateEvidence) {
        self.consider(name, "not-considered.no-physical-shape");
        self.decisions
            .get_mut(name)
            .expect("just inserted")
            .cross_crate = evidence;
    }

    fn record_callers(
        &mut self,
        table: &BTreeMap<JsName, LinkItem>,
        name: &JsName,
        callers: &BTreeSet<JsName>,
    ) {
        let Some(decision) = self.decisions.get_mut(name) else {
            return;
        };
        let producer = table.get(name).map(|item| item.crate_name.as_str());
        let caller_crates: BTreeSet<String> = callers
            .iter()
            .filter_map(|caller| table.get(caller))
            .map(|item| item.crate_name.clone())
            .collect();
        decision.cross_crate.caller_count = callers.len().min(u32::MAX as usize) as u32;
        decision.cross_crate.crosses_crate =
            producer.is_some_and(|producer| caller_crates.iter().any(|caller| caller != producer));
        decision.cross_crate.caller_crates = caller_crates.into_iter().collect();
    }

    fn summary_validated(&self, name: &JsName) -> bool {
        self.decisions
            .get(name)
            .is_some_and(|decision| decision.cross_crate.summary_validated)
    }

    fn record_reason(&mut self, name: &JsName, reason: impl Into<String>) {
        let reason = reason.into();
        let reason = terminal_reason(&reason);
        self.consider(name, reason.clone());
        let decision = self.decisions.get_mut(name).expect("just inserted");
        decision.selected = false;
        decision.reason = Some(reason);
    }

    fn decline(&mut self, name: JsName, decline: Decline) {
        self.record_reason(&name, decline.code());
        self.declined.insert(name, decline);
    }

    fn select(&mut self, name: &JsName, sig: &FlatSig) {
        self.consider(name, "not-considered.no-physical-shape");
        let decision = self.decisions.get_mut(name).expect("just inserted");
        decision.shape = Some(signature_shape(sig));
        decision.selected = true;
        decision.reason = None;
    }

    /// Whether the plan changes `name`'s own signature or a call it makes.
    ///
    /// An item that declines *after* being rewritten is what makes the driver restart with an
    /// empty plan: its callers would otherwise print calls in a shape the fallback text does not
    /// accept.
    pub fn touches(&self, name: &JsName) -> bool {
        self.touched.contains(name)
    }

    /// The names the registers were allocated, register 1 first. What the printer spells a
    /// [`OpKind::SpillRead`]/[`OpKind::SpillWrite`] as.
    pub fn spill_names(&self) -> &[JsName] {
        &self.spill_names
    }

    /// The selected signature of a direct call. Canonical and private entry points share the
    /// semantic item id; only direct calls modeled by this plan consult it.
    pub(crate) fn call_signature(&self, item: ItemId) -> Option<&FlatSig> {
        self.flat.get(&item)
    }

    /// The selected signature when `name` is the body whose physical declaration changes.
    pub(crate) fn target_signature(&self, name: &JsName, item: ItemId) -> Option<&FlatSig> {
        (self.names.get(&item) == Some(name))
            .then(|| self.flat.get(&item))
            .flatten()
    }

    /// Whether `name` is a private flat body behind a canonical wrapper.
    pub(crate) fn has_canonical_wrapper(&self, name: &JsName) -> bool {
        self.clones.contains_key(name)
    }

    /// Direct-call retargets selected for `caller`.
    pub(crate) fn call_retargets(&self, caller: &JsName) -> &crate::lower::CallRetargets {
        self.call_retargets
            .get(caller)
            .unwrap_or(crate::lower::EMPTY_CALL_RETARGETS)
    }

    /// The private clone names and their canonical sources.
    pub(crate) fn clones(&self) -> &BTreeMap<JsName, JsName> {
        &self.clones
    }

    /// Adds the non-emitting placeholders and JIR functions for private flat clones.
    pub(crate) fn install_clones(
        &self,
        table: &mut BTreeMap<JsName, LinkItem>,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ) {
        for (clone, source) in &self.clones {
            let Some(item) = table.get(source).cloned() else {
                continue;
            };
            table.insert(
                clone.clone(),
                crate::plan::representation::clone_link_item(item, clone),
            );
            let Some(func) = funcs.get(source).cloned() else {
                continue;
            };
            funcs.insert(clone.clone(), func);
        }
    }

    /// The synthesized spill-register items, one module-scope `let` each.
    ///
    /// They are items like any other — the reachability walk follows a reference to one, `order`
    /// places a binding before the items that name it, and `print_units` prints it — which is why
    /// the plan does not have to reach into the printer at all.
    pub fn spill_items(&self) -> Vec<LinkItem> {
        self.spill_names
            .iter()
            .map(|name| {
                let name = name.clone();
                LinkItem {
                    js: format!("\nlet {};\n", name.as_str()),
                    name,
                    kind: ItemKind::Static,
                    refs: Default::default(),
                    zombies: Vec::new(),
                    linkage: Linkage::internal(),
                    debug_path: "flat direct-call ABI spill register".to_owned(),
                    crate_name: "generated".to_owned(),
                    order: None,
                    map: ItemMap::default(),
                    dts: None,
                    optimization: Default::default(),
                    late_ir: Default::default(),
                    baseline_shadow: Default::default(),
                    jir: Default::default(),
                }
            })
            .collect()
    }

    /// The sidecar's `abi` block, or `None` when there was nothing to plan.
    ///
    /// `applied` says whether the plan is the one the program was printed against. It is false
    /// under `js-opt=report`, which plans so it can report the plan and then throws it away.
    pub fn json(&self, applied: bool) -> Option<String> {
        if self.flat.is_empty() && self.declined.is_empty() {
            return None;
        }
        let mut rows: Vec<(&JsName, &FlatSig)> = self
            .flat
            .iter()
            .filter_map(|(item, sig)| Some((self.names.get(item)?, sig)))
            .collect();
        rows.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\"applied\":{applied},\"registers\":{},\"flat\":[",
            self.registers
        );
        for (index, (name, sig)) in rows.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, name.as_str());
            out.push_str(",\"params\":[");
            for (position, param) in sig.params.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}", param.as_ref().map_or(0, FlatParam::fields));
            }
            let _ = write!(
                out,
                "],\"result\":{},\"spill\":{},\"canonical_wrapper\":{}}}",
                sig.result.as_ref().map_or(0, FlatResult::fields),
                sig.spill,
                self.clones.contains_key(*name),
            );
        }
        out.push_str("],\"declined\":[");
        for (index, (name, decline)) in self.declined.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, name.as_str());
            out.push_str(",\"reason\":");
            push_string(&mut out, decline.code());
            out.push('}');
        }
        out.push_str("]}");
        Some(out)
    }

    /// Rewrites every flat callee's signature and then every caller's calls, in place.
    ///
    /// Callees first: a caller's rewrite is written against the signature the callee already has.
    /// A function whose rewrite does not verify becomes a [`Decline`], which the driver turns into
    /// a retry with an empty plan — a callee's new shape is applied to all of its callers or to
    /// none of them.
    pub fn rewrite(
        &self,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
        table: &BTreeMap<JsName, LinkItem>,
    ) {
        if self.flat.is_empty() {
            return;
        }
        for (name, entry) in funcs.iter_mut() {
            let Ok(func) = entry else { continue };
            if self.names.get(&func.item) == Some(name) {
                if let Some(sig) = self.flat.get(&func.item) {
                    rewrite_callee(func, sig);
                }
            }
        }
        for (name, entry) in funcs.iter_mut() {
            if !self.touched.contains(name) {
                continue;
            }
            let Ok(func) = entry else { continue };
            let types = type_table(table, name);
            rewrite_calls(func, self, types);
            let Some(types) = types else {
                continue;
            };
            // Under the carriers the values already have: a committed representation plan
            // (Phase 4, B1) runs before this rewrite and its carriers must not decline it.
            if let Err(err) = func.verify_carriers(types, crate::opt::observed_carriers(func)) {
                *entry = Err(Decline::Lower(LowerDecline::Other {
                    op: OpClass::Region,
                    detail: format!("plan/abi_flat: {err}"),
                }));
            }
        }
    }
}

/// The type table of the object `name` was read from.
fn type_table<'a>(table: &'a BTreeMap<JsName, LinkItem>, name: &JsName) -> Option<&'a TypeTable> {
    table
        .get(name)
        .and_then(|item| item.jir.interns())
        .map(|interns| &interns.types)
}

/// Validates the eager physical summary against the decoded body before a planner may consume it.
fn physical_evidence(
    table: &BTreeMap<JsName, LinkItem>,
    name: &JsName,
) -> (CrossCrateEvidence, Result<(), &'static str>) {
    let Some(item) = table.get(name) else {
        return (
            CrossCrateEvidence::default(),
            Err("validation.summary-item"),
        );
    };
    let mut evidence = CrossCrateEvidence {
        body_decoded: item.jir.body.is_some(),
        producer_crate: Some(item.crate_name.clone()),
        ..CrossCrateEvidence::default()
    };
    let Some(index) = item.jir.summary.as_ref() else {
        return (evidence, Err("validation.summary-missing"));
    };
    let Some(physical) = index.physical.as_ref() else {
        return (evidence, Err("validation.summary-missing"));
    };
    evidence.summary_available = true;
    let Some(body) = item.jir.body.as_ref() else {
        return (evidence, Err("validation.summary-body-unavailable"));
    };
    let Some(interns) = item.jir.interns() else {
        return (evidence, Err("validation.summary-interns-unavailable"));
    };
    let semantic_hash = summary::semantic_hash(body, interns);
    match physical.validate(body, &interns.types, semantic_hash) {
        Ok(()) => {
            evidence.summary_validated = true;
            (evidence, Ok(()))
        }
        Err(validation) => (evidence, Err(validation.code())),
    }
}

/// Plans the flat direct-call ABI over every function the lane built.
///
/// `funcs` is phase A's output — a `Func` per offered function, or the decline that stopped it. A
/// declined entry is *not* a lane item: it keeps the baseline text, so what it references counts
/// against the plan rather than for it.
/// Plans the flat ABI while retaining the baseline signature of `excluded` items.
///
/// A representation-version clone deliberately shares its Rust [`ItemId`] with the public body.
/// The private clone is already specialized by its carrier ABI; flattening that shared identity
/// would also change the retained baseline body, so version sources are excluded as one unit.
pub fn plan_excluding(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    opts: &Opts,
    excluded: &BTreeSet<ItemId>,
) -> AbiPlan {
    let mut plan = AbiPlan::default();
    // The rewrite only pays off once the escape analysis takes the rebuilt aggregate apart again,
    // so the plan is part of `sroa` and is made exactly when `sroa` runs; `js-jir-flat-abi=off`
    // bisects it out from under `js-opt=on`. Under `js-opt=report` the plan is made and reported
    // but never applied — the driver empties it before phase B, since `report` owes the program
    // no rewrite at all (the lane's pass mapping, D1).
    if !opts.flat_abi || !opts.analyzes(crate::opts::OptimizerPass::Sroa) {
        return plan;
    }
    for (name, func) in funcs {
        let aggregate_body = table
            .get(name)
            .and_then(|item| item.jir.summary.as_ref())
            .is_some_and(|summary| summary.class.contains(ItemClass::AGGREGATE));
        // A pure forwarding helper may never construct or project the aggregate itself, so the
        // body class is empty even though its signature is an essential physical edge. Validate
        // those summaries as well; otherwise a producer -> forwarder -> leaf chain is cut solely
        // because the middle body did exactly the inexpensive thing the plan wants to preserve.
        let aggregate_signature = func.as_ref().ok().is_some_and(|func| {
            type_table(table, name).is_some_and(|types| {
                func.params.iter().any(|param| {
                    let ty = func.values[param.0 as usize].ty;
                    slice_param(func, types, *param, ty).is_some()
                        || flat_param(types, ty, true).is_some()
                }) || flat_result(types, func.sig.result).is_some()
            })
        });
        if !aggregate_body && !aggregate_signature {
            continue;
        }
        let (evidence, validation) = physical_evidence(table, name);
        plan.record_evidence(name, evidence);
        match func {
            Ok(_) => match validation {
                Ok(()) => plan.consider(name, "not-considered.no-physical-shape"),
                Err(reason) => plan.record_reason(name, reason),
            },
            Err(decline) => plan.record_reason(name, decline.code()),
        }
    }
    let lowered: BTreeMap<ItemId, &JsName> = funcs
        .iter()
        .filter_map(|(name, func)| Some((func.as_ref().ok()?.item, name)))
        .collect();
    let by_name: BTreeMap<&str, ItemId> = lowered
        .iter()
        .map(|(item, name)| (name.as_str(), *item))
        .collect();

    // What the program mentions other than by a plain direct call: the `refs` of everything the
    // lane did not lower, plus every item-naming op of everything it did.
    let mut scan = RefScan::default();
    scan.named
        .extend(refs::named_by_unlowered(table, funcs, &by_name));
    // The callers of each item, and whether any call site keeps its result.
    let mut callers: BTreeMap<ItemId, BTreeSet<JsName>> = BTreeMap::new();
    let mut result_used: BTreeSet<ItemId> = BTreeSet::new();
    // A `#[track_caller]` body decides whether each callee takes its `$loc` by checking the
    // callee's recorded parameter count against the call's argument count
    // (`lower/js/call.rs::callee_takes_caller_location`). That count comes from the summary index,
    // which the plan does not rewrite, so a callee called from such a body keeps its shape.
    let mut pinned_by_tracked_caller: BTreeSet<ItemId> = BTreeSet::new();
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        let types = type_table(table, name);
        for block in &func.blocks {
            for op in &block.ops {
                refs::scan_op(func, op, types, &mut scan);
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } = &op.kind
                {
                    callers.entry(*item).or_default().insert(name.clone());
                    if op.result.is_some() {
                        result_used.insert(*item);
                    }
                    if func.sig.abi.caller_location {
                        pinned_by_tracked_caller.insert(*item);
                    }
                }
            }
        }
    }
    for (item, item_callers) in &callers {
        if let Some(name) = lowered.get(item) {
            plan.record_callers(table, name, item_callers);
        }
    }

    let mut ineligible = scan.ineligible();
    ineligible.extend(excluded.iter().copied());

    // The optimistic physical shape of every lowered parameter. Result selection consults this
    // table so a product returned by one direct call may flow whole into another call whose
    // parameter will be flattened. The fixed point below still decides whether that destination
    // parameter actually survives; an ineligible or escaping destination therefore removes the
    // producer's result lanes instead of leaving a reconstruction at the boundary.
    let potential_params: BTreeMap<ItemId, Vec<Option<FlatParam>>> = funcs
        .iter()
        .filter_map(|(name, func)| {
            let func = func.as_ref().ok()?;
            let types = type_table(table, name)?;
            let shapes = func
                .params
                .iter()
                .map(|param| {
                    if func.values[param.0 as usize].carrier != Carrier::Baseline {
                        return None;
                    }
                    let ty = func.values[param.0 as usize].ty;
                    slice_param(func, types, *param, ty).or_else(|| flat_param(types, ty, true))
                })
                .collect();
            Some((func.item, shapes))
        })
        .collect();

    // A lowered item whose *baseline* text names a candidate but whose JIR makes no direct call to
    // it is naming it in some way this scan did not model — the two lowerings must not disagree
    // about a name whose calling convention is about to change.
    for (name, item) in table {
        if !funcs.get(name).is_some_and(Result::is_ok) {
            continue;
        }
        for referenced in &item.refs {
            let Some(id) = by_name.get(referenced.as_str()) else {
                continue;
            };
            if !callers.get(id).is_some_and(|set| set.contains(name)) {
                ineligible.insert(*id);
            }
        }
    }

    let interns = table
        .iter()
        .filter_map(|(name, item)| item.jir.interns().map(|interns| (name.clone(), interns)))
        .collect();
    let graph = crate::analysis::callgraph::build(funcs, &interns);
    let effects = crate::analysis::effects::compute(funcs, &interns, &graph);

    // The candidates, with the shape each parameter would take and how the body uses it.
    let mut candidates: BTreeMap<ItemId, Candidate> = BTreeMap::new();
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        if !plan.summary_validated(name) {
            continue;
        }
        let Some(types) = type_table(table, name) else {
            continue;
        };
        let mut shapes = potential_params
            .get(&func.item)
            .cloned()
            .unwrap_or_else(|| vec![None; func.params.len()]);
        let mut result_forwards = BTreeSet::new();
        let mut result_forward_sites = 0;
        let mut result_hot_forward = false;
        let mut result_reason = None;
        let result = flat_result(types, func.sig.result).filter(|result| {
            if !returns_baseline(func, result.ty) {
                result_reason = Some("legality.result-carrier");
                return false;
            }
            let Some(direct_callers) = callers.get(&func.item).filter(|set| !set.is_empty()) else {
                result_reason = Some("not-considered.no-direct-caller");
                return false;
            };
            let mut observed_field = false;
            for caller in direct_callers {
                let Some(observed) = caller_accepts_result(
                    table,
                    funcs,
                    caller,
                    func.item,
                    result,
                    &potential_params,
                ) else {
                    result_reason = Some("boundary.result-use");
                    return false;
                };
                observed_field |= observed.read || !observed.forwards.is_empty();
                result_forwards.extend(observed.forwards);
                result_forward_sites += observed.forward_sites;
                result_hot_forward |= observed.hot_forward;
            }
            if !observed_field {
                result_reason = Some("not-considered.result-unused");
            }
            observed_field
        });
        if shapes.iter().all(Option::is_none) && result.is_none() {
            if let Some(reason) = result_reason {
                plan.record_reason(name, reason);
            }
            continue;
        }
        let item = table.get(name);
        let semantic = item
            .and_then(|item| item.jir.body.as_ref())
            .map(|body| body.semantic);
        let refusal = refuses(
            item.map(|item| &item.linkage),
            semantic,
            func,
            !opts.chunks.is_empty(),
            &ineligible,
        )
        .or_else(|| {
            pinned_by_tracked_caller
                .contains(&func.item)
                .then_some(BoundaryDecline::TrackedCaller)
        });
        let canonical = refusal.is_some_and(|reason| {
            matches!(
                reason,
                BoundaryDecline::Exported | BoundaryDecline::NonCallReference
            ) && canonical_clone_allowed(
                semantic,
                func,
                !opts.chunks.is_empty(),
                excluded.contains(&func.item),
                pinned_by_tracked_caller.contains(&func.item),
            ) && callers
                .get(&func.item)
                .is_some_and(|callers| !callers.is_empty())
        });
        if let Some(reason) = refusal.filter(|_| !canonical) {
            plan.decline(name.clone(), Decline::Boundary(reason));
            continue;
        }
        let kinds: Vec<Option<ParamKind>> = shapes
            .iter()
            .map(|shape| {
                shape.as_ref().and_then(|shape| {
                    if shape.i64_pair {
                        None
                    } else if shape.slice {
                        Some(ParamKind::Slice)
                    } else if shape.by_ref {
                        Some(ParamKind::ByRef)
                    } else {
                        Some(ParamKind::ByValue)
                    }
                })
            })
            .collect();
        let uses = param_use::summarize(func, types, &kinds);
        // Copy-back runs only after a normal return. A direct or transitive throw must not
        // hide writes in a reconstructed private aggregate from the caller's cleanup code.
        if !effects
            .usable(name)
            .is_some_and(|summary| !summary.may_unwind)
            && shapes.iter().zip(&uses).any(|(shape, use_)| {
                shape
                    .as_ref()
                    .is_some_and(|shape| shape.by_ref && shape.mutable)
                    && use_.written
            })
        {
            plan.decline(
                name.clone(),
                Decline::Boundary(BoundaryDecline::ExceptionalCopyBack),
            );
            continue;
        }
        // This tagged tranche ends at a leaf scalar consumer. `Option` into a closure-taking
        // helper and `Result` into `ControlFlow` are tag-to-call/tag-to-tag transforms; selecting
        // them before recursive physical products and exact ABI costing merely moves (and often
        // enlarges) the materialization. A function that forwards the whole parameter is admitted
        // separately by `tagged_chain_members`, so direct chains and SCCs can still reach a leaf.
        let tagged_leaf_consumer = scalar_leaf_result(types, func.sig.result)
            && func
                .blocks
                .iter()
                .all(|block| block.ops.iter().all(|op| scalar_leaf_op(types, func, op)));
        // A `&mut T` the body never writes needs no copy-back and no spill register.
        for (shape, use_) in shapes.iter_mut().zip(&uses) {
            // Tagged parameters are rebound as scalar expressions in phase C, not as an
            // addressable enum object. Field-only reads and whole-value forwards are safe;
            // in-place mutation keeps the canonical ABI until a storage plan can represent it
            // without changing aliasing.
            if shape.as_ref().is_some_and(|shape| shape.tagged.is_some()) && use_.mutated {
                *shape = None;
                continue;
            }
            if let Some(shape) = shape {
                shape.written = use_.written;
            }
        }
        // Every caller has to be able to spell the fields the plan chose out of its own crate's
        // type table: a `TypeId` is shared between crates, a table is not.
        if !callers
            .get(&func.item)
            .into_iter()
            .flatten()
            .all(|caller| caller_agrees(table, caller, &shapes))
        {
            plan.decline(
                name.clone(),
                Decline::Boundary(BoundaryDecline::NonCallReference),
            );
            continue;
        }
        candidates.insert(
            func.item,
            Candidate {
                name: name.clone(),
                returns_zst: types.is_zst(func.sig.result),
                uses,
                shapes,
                result,
                result_forwards,
                result_forward_sites,
                result_hot_forward,
                tagged_leaf_consumer,
                canonical,
            },
        );
    }

    // Tagged products are deliberately admitted only as part of a real producer-to-consumer
    // chain. Flattening every isolated `Option`/`Result` helper is usually negative after gzip:
    // the extra tag/payload parameters and spill spelling cost more than one short-lived object.
    // A returned tagged value forwarded whole into another prepared parameter is the concrete
    // proof that scalar lanes cross a boundary. This first tagged tranche ends at a leaf scalar
    // consumer; tag-to-tag and closure-taking transforms keep the canonical object until the
    // exact compiler-link artifact cost model can prove otherwise.
    let tagged = tagged_chain_members(&candidates);
    for (item, candidate) in &mut candidates {
        if candidate
            .result
            .as_ref()
            .is_some_and(|result| result.tagged.is_some())
            && !tagged.results.contains(item)
        {
            candidate.result = None;
            candidate.result_forwards.clear();
        }
        for (position, shape) in candidate.shapes.iter_mut().enumerate() {
            if shape.as_ref().is_some_and(|shape| shape.tagged.is_some())
                && !tagged.params.contains(&(*item, position))
            {
                *shape = None;
            }
        }
    }

    let flat = close_over_forwards(&candidates);

    // What survived, and what it costs in registers. A boundary that needs its canonical body
    // receives one collision-safe private flat version; only the direct callers modeled above
    // retarget to it.
    let mut taken: BTreeSet<String> = table.keys().map(|name| name.as_str().to_owned()).collect();
    taken.extend(
        table
            .values()
            .flat_map(|item| item.refs.iter().map(|name| name.as_str().to_owned())),
    );
    for (item, mut candidate) in candidates {
        if !result_forwards_survive(&candidate.result_forwards, &flat) {
            candidate.result = None;
        }
        let params: Vec<Option<FlatParam>> = candidate
            .shapes
            .into_iter()
            .zip(&flat[&item])
            .map(|(shape, keep)| shape.filter(|_| *keep))
            .collect();
        if params.iter().all(Option::is_none) && candidate.result.is_none() {
            plan.decline(
                candidate.name,
                Decline::Boundary(BoundaryDecline::ParamEscapes),
            );
            continue;
        }
        let mut sig = FlatSig {
            params,
            result: candidate.result,
            spill: 0,
            returns_out_field: false,
        };
        let outs = sig.out_fields().len() as u32;
        // The primary return is free only when the callee has no result of its own to carry and
        // no caller binds one.
        sig.returns_out_field = sig.result.is_none()
            && outs > 0
            && candidate.returns_zst
            && !result_used.contains(&item);
        sig.spill = sig.result_spills() + outs - u32::from(sig.returns_out_field);
        plan.registers = plan.registers.max(sig.spill);
        let target = if candidate.canonical {
            if let Err(decline) = crate::plan::versions::budget(1) {
                plan.decline(candidate.name, decline);
                continue;
            }
            let clone = match crate::plan::versions::allocate_version_name_owned(
                candidate.name.as_str(),
                &taken,
            ) {
                Ok(clone) => clone,
                Err(decline) => {
                    plan.decline(candidate.name, decline);
                    continue;
                }
            };
            taken.insert(clone.as_str().to_owned());
            plan.clones.insert(clone.clone(), candidate.name.clone());
            for caller in callers.get(&item).into_iter().flatten() {
                plan.call_retargets
                    .entry(caller.clone())
                    .or_default()
                    .insert(item, clone.clone());
            }
            clone
        } else {
            candidate.name.clone()
        };
        plan.touched.insert(target.clone());
        plan.touched
            .extend(callers.get(&item).into_iter().flatten().cloned());
        if let Some(item_callers) = callers.get(&item) {
            plan.callers.insert(item, item_callers.clone());
        }
        plan.select(&candidate.name, &sig);
        plan.names.insert(item, target);
        plan.flat.insert(item, sig);
    }
    // A private body begins as a clone of its canonical source, including every direct call the
    // source makes. Those calls have already been reshaped by `rewrite_calls`; give the clone the
    // same retargets so the expanded arguments reach the matching private callees rather than a
    // canonical entry point. This matters for chains such as allocator wrappers, where both ends
    // retain externally named canonical bodies.
    inherit_clone_retargets(&mut plan);
    match allocate_spill_names(plan.registers, table) {
        Some(names) => plan.spill_names = names,
        // Nowhere left to put a register, so the program keeps its own names and the plan keeps
        // nothing: every callee it had flattened is recorded as refused instead.
        None => {
            let mut declined = plan.declined;
            let mut decisions = plan.decisions;
            for name in plan.names.into_values() {
                let decline = Decline::Boundary(BoundaryDecline::GeneratedNameCollision);
                if let Some(decision) = decisions.get_mut(&name) {
                    decision.selected = false;
                    decision.reason = Some(terminal_reason(decline.code()));
                }
                declined.insert(name, decline);
            }
            for decision in decisions.values_mut().filter(|decision| decision.selected) {
                decision.selected = false;
                decision.reason = Some(terminal_reason(
                    Decline::Boundary(BoundaryDecline::GeneratedNameCollision).code(),
                ));
            }
            plan = AbiPlan {
                declined,
                decisions,
                ..AbiPlan::default()
            };
        }
    }
    plan
}

/// Maps the compiler-wide decline vocabulary onto the physical report's terminal-state contract.
fn terminal_reason(reason: &str) -> String {
    let category = reason.split('.').next().unwrap_or_default();
    let terminal = match category {
        "selected" | "not-considered" | "legality" | "boundary" | "cost" | "lower"
        | "validation" => return reason.to_owned(),
        "body" | "type" | "observe" | "op" | "kernel" | "std" | "drop" | "format" | "iterator"
        | "async" => "legality",
        "encode" => "lower",
        _ => "validation",
    };
    format!("{terminal}.{reason}")
}

fn signature_shape(sig: &FlatSig) -> String {
    let mut parts = Vec::new();
    for param in sig.params.iter().flatten() {
        let kind = if param.slice {
            "slice"
        } else if param.i64_pair {
            "i64-pair"
        } else if param.tagged.is_some() {
            "tagged"
        } else if param.recursive.is_some() {
            "recursive-product"
        } else {
            "product"
        };
        parts.push(format!("{kind}({})", param.fields()));
    }
    if let Some(result) = &sig.result {
        let kind = if result.i64_pair {
            "i64-pair-result"
        } else if result.tagged.is_some() {
            "tagged-result"
        } else if result.recursive.is_some() {
            "recursive-product-result"
        } else {
            "product-result"
        };
        parts.push(format!("{kind}({})", result.fields()));
    }
    parts.join("+")
}

/// Copies a canonical source's already selected direct-call targets to its private body.
fn inherit_clone_retargets(plan: &mut AbiPlan) {
    for (clone, source) in plan.clones.clone() {
        let Some(retargets) = plan.call_retargets.get(&source).cloned() else {
            continue;
        };
        plan.call_retargets.insert(clone, retargets);
    }
}

/// A name for each of `registers` spill registers that the program does not already use.
///
/// A register is a module-scope `let`, and the program is free to bind `$r1` itself — an
/// `#[unsafe(export_name = "$r1")]` function is all it takes, and a `js!` block may mention any
/// name at all. So the preferred spelling (`emit::spill_name`) is only a starting point: a name
/// already spoken for anywhere in the program — defined by an item, or mentioned by one — is
/// stepped past by suffixing `$1`, `$2`, ... The suffixed names cannot collide with the `$h`
/// names `link::stub_missing` synthesizes either, because none of them contains `$h`.
///
/// `None` when a register ran out of candidates, which refuses the plan whole.
fn allocate_spill_names(registers: u32, table: &BTreeMap<JsName, LinkItem>) -> Option<Vec<JsName>> {
    if registers == 0 {
        return Some(Vec::new());
    }
    // Every name the program spells: what the item table defines (a `link_dedup` winner included,
    // since dedup only ever keeps a name that is already here) plus every identifier an item's
    // text mentions.
    let mut taken: BTreeSet<&str> = table.keys().map(JsName::as_str).collect();
    taken.extend(
        table
            .values()
            .flat_map(|item| item.refs.iter().map(JsName::as_str)),
    );
    let mut names = Vec::with_capacity(registers as usize);
    for register in 1..=registers {
        let base = spill_name(register);
        let name = std::iter::once(base.clone())
            .chain((1..64).map(|suffix| format!("{base}${suffix}")))
            .find(|name| !taken.contains(name.as_str()) && !names.iter().any(|n| n == name))?;
        names.push(name);
    }
    Some(names.into_iter().map(JsName::new).collect())
}

/// One callee the plan is still considering.
struct Candidate {
    name: JsName,
    returns_zst: bool,
    shapes: Vec<Option<FlatParam>>,
    result: Option<FlatResult>,
    /// Direct-call parameter lanes that must survive for the result to remain field-only.
    result_forwards: BTreeSet<(ItemId, usize)>,
    /// Number of static producer-call sites whose result crosses one of those edges.
    result_forward_sites: u32,
    /// Whether any such producer call is in a cyclic CFG region.
    result_hot_forward: bool,
    /// Whether this function consumes a tagged parameter without another call/product boundary.
    tagged_leaf_consumer: bool,
    uses: Vec<ParamUse>,
    canonical: bool,
}

/// Tagged result and leaf-parameter sites belonging to a whole-value direct-call edge.
#[derive(Default)]
struct TaggedChains {
    results: BTreeSet<ItemId>,
    params: BTreeSet<(ItemId, usize)>,
}

/// Finds the minimal tagged ABI subgraph rooted at a returned value forwarded into a call.
///
/// Fixed products already have enough local wins to justify field-only result lowering. Tagged
/// values pay for a discriminant and optional payload spelling, so merely destructuring one at a
/// call site is not sufficient evidence. A whole-value forward is both the requested NS4 case
/// and a structural proof that canonical materialization would otherwise sit between two direct
/// calls. A bounded chain or SCC is admitted when every forward remains tagged and at least one
/// path reaches a scalar leaf; a closed forwarding cycle with no consuming leaf has no evidence
/// of removing a materialization and stays canonical.
fn tagged_chain_members(candidates: &BTreeMap<ItemId, Candidate>) -> TaggedChains {
    let mut eligible: BTreeSet<(ItemId, usize)> = candidates
        .iter()
        .flat_map(|(item, candidate)| {
            candidate
                .shapes
                .iter()
                .enumerate()
                .filter_map(move |(position, shape)| {
                    let use_ = candidate.uses.get(position)?;
                    (shape.as_ref().is_some_and(|shape| shape.tagged.is_some())
                        && !use_.escapes
                        && !use_.mutated
                        && (candidate.tagged_leaf_consumer || !use_.forwards.is_empty()))
                    .then_some((*item, position))
                })
        })
        .collect();
    loop {
        let before = eligible.len();
        let snapshot = eligible.clone();
        eligible.retain(|(item, position)| {
            let candidate = &candidates[item];
            let use_ = &candidate.uses[*position];
            candidate.tagged_leaf_consumer
                || (!use_.forwards.is_empty()
                    && use_.forwards.iter().all(|target| snapshot.contains(target)))
        });
        if eligible.len() == before {
            break;
        }
    }

    let mut reaches_leaf: BTreeSet<(ItemId, usize)> = eligible
        .iter()
        .copied()
        .filter(|(item, _)| candidates[item].tagged_leaf_consumer)
        .collect();
    loop {
        let before = reaches_leaf.len();
        for key in &eligible {
            let use_ = &candidates[&key.0].uses[key.1];
            if !use_.forwards.is_empty()
                && use_.forwards.iter().all(|target| eligible.contains(target))
                && use_
                    .forwards
                    .iter()
                    .any(|target| reaches_leaf.contains(target))
            {
                reaches_leaf.insert(*key);
            }
        }
        if reaches_leaf.len() == before {
            break;
        }
    }

    let mut chains = TaggedChains::default();
    let mut pending = Vec::new();
    for (item, candidate) in candidates {
        if candidate
            .result
            .as_ref()
            .is_some_and(|result| result.tagged.is_some())
            && !candidate.result_forwards.is_empty()
            && (candidate.result_forward_sites >= 2 || candidate.result_hot_forward)
            && candidate
                .result_forwards
                .iter()
                .all(|target| reaches_leaf.contains(target))
        {
            chains.results.insert(*item);
            pending.extend(candidate.result_forwards.iter().copied());
        }
    }

    while let Some((item, position)) = pending.pop() {
        let Some(candidate) = candidates.get(&item) else {
            continue;
        };
        if !reaches_leaf.contains(&(item, position)) || !chains.params.insert((item, position)) {
            continue;
        }
        if !candidate.tagged_leaf_consumer {
            pending.extend(candidate.uses[position].forwards.iter().copied());
        }
    }
    chains
}

/// Whether an exported or indirect canonical entry point may coexist with one private flat body.
fn canonical_clone_allowed(
    semantic: Option<BodySemantic>,
    func: &Func,
    chunked: bool,
    excluded: bool,
    tracked_caller: bool,
) -> bool {
    !chunked
        && !excluded
        && !tracked_caller
        && func.sig.abi.spread.is_none()
        && !func.sig.abi.caller_location
        && !matches!(semantic, Some(BodySemantic::DropGlue(_)))
        && !matches!(
            semantic,
            Some(BodySemantic::Standard(operation))
                if crate::lower::semantic_ops::fmt_kernel::is_format_machinery(operation)
        )
}

/// Why the callee may not have its calling convention changed, or `None`.
fn refuses(
    linkage: Option<&Linkage>,
    semantic: Option<BodySemantic>,
    func: &Func,
    chunked: bool,
    ineligible: &BTreeSet<ItemId>,
) -> Option<BoundaryDecline> {
    // A spill register is one module-scope binding; two chunks would each want their own.
    if chunked {
        return Some(BoundaryDecline::Chunks);
    }
    if linkage? != &Linkage::internal() {
        return Some(BoundaryDecline::Exported);
    }
    if func.sig.abi.spread.is_some() || func.sig.abi.caller_location {
        return Some(BoundaryDecline::SpreadAbi);
    }
    // Drop glue is named by the *type* it drops, which a host lowering resolves with no op saying
    // so, so nothing here could prove its mentions.
    if matches!(semantic, Some(BodySemantic::DropGlue(_))) {
        return Some(BoundaryDecline::NonCallReference);
    }
    // Format machinery keeps the exact call shape the call-site format kernel recognizes
    // (`lower/semantic_ops/fmt_kernel.rs`): a flattened `Arguments::new` caller would no longer
    // match a proven site, and the items are about to be dropped wherever the kernel commits.
    if matches!(
        semantic,
        Some(BodySemantic::Standard(operation))
            if crate::lower::semantic_ops::fmt_kernel::is_format_machinery(operation)
    ) {
        return Some(BoundaryDecline::NonCallReference);
    }
    if ineligible.contains(&func.item) {
        return Some(BoundaryDecline::NonCallReference);
    }
    None
}

/// The flat shape of a parameter of type `ty`, or `None` when it has none.
///
/// A candidate aggregate by value, or a reference to one. The field types come from
/// [`crate::opt::candidates::shape`], so the flat ABI and the escape analysis agree about which
/// aggregates have fields worth naming — which is the point: the callee's rebuilt aggregate is
/// only worth building because the escape analysis takes it apart again.
fn flat_param(table: &TypeTable, ty: TypeId, written: bool) -> Option<FlatParam> {
    let (aggregate, by_ref, mutable) = match table.get(table.peeled(ty)).map(|d| &d.kind) {
        Some(TypeKind::Reference { pointee, mutable }) => (*pointee, true, *mutable),
        _ => (ty, false, false),
    };
    if !by_ref {
        if crate::lower::js::i64_pair::is_i64(table, aggregate) {
            return Some(FlatParam {
                ty: aggregate,
                field_tys: vec![ty],
                passed: vec![0],
                by_ref: false,
                mutable: false,
                written: false,
                slice: false,
                i64_pair: true,
                tagged: None,
                recursive: None,
            });
        }
        if let Some(tagged) = tagged_shape(table, aggregate) {
            let field_tys = tagged.lane_types();
            let passed = (0..field_tys.len() as u32).collect();
            return Some(FlatParam {
                ty: aggregate,
                field_tys,
                passed,
                by_ref: false,
                mutable: false,
                written,
                slice: false,
                i64_pair: false,
                tagged: Some(tagged),
                recursive: None,
            });
        }
        if let Some(recursive) = recursive_shape(table, aggregate) {
            let field_tys = recursive.lane_types();
            let passed = (0..field_tys.len() as u32).collect();
            return Some(FlatParam {
                ty: aggregate,
                field_tys,
                passed,
                by_ref: false,
                mutable: false,
                written,
                slice: false,
                i64_pair: false,
                tagged: None,
                recursive: Some(recursive),
            });
        }
    }
    let (field_tys, passed) = flat_shape(table, aggregate)?;
    // The rewrite spells the reference as a borrow of the rebuilt aggregate with an empty path,
    // which the verifier allows only for a value with storage to point at.
    if by_ref && !table.is_indirect(aggregate) {
        return None;
    }
    Some(FlatParam {
        ty: aggregate,
        field_tys,
        passed,
        by_ref,
        mutable,
        written,
        slice: false,
        i64_pair: false,
        tagged: None,
        recursive: None,
    })
}

/// A slice pointer that can travel as its JavaScript buffer, offset, and scalar length.
fn slice_param(func: &Func, table: &TypeTable, param: ValueId, ty: TypeId) -> Option<FlatParam> {
    let (pointee, mutable) = match table
        .get(table.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Reference { pointee, mutable })
        | Some(TypeKind::RawPointer { pointee, mutable }) => (*pointee, *mutable),
        _ => return None,
    };
    if !matches!(
        table
            .get(table.peeled(pointee))
            .map(|descriptor| &descriptor.kind),
        Some(TypeKind::Slice(_))
    ) {
        return None;
    }
    let usize_ty = func
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op.kind {
            OpKind::Unary {
                op: crate::jir::semantic::UnaryOp::PtrMetadata,
                v,
                ..
            } if v == param => op.result.and_then(|result| func.ty(result)),
            _ => None,
        })
        .or_else(|| {
            table.descriptors().iter().find_map(|descriptor| {
                matches!(
                    descriptor.kind,
                    TypeKind::Integer {
                        signed: false,
                        bits: 32
                    }
                )
                .then_some(descriptor.id)
            })
        })?;
    Some(FlatParam {
        ty,
        // The first type is an opaque physical buffer lane. It deliberately retains the slice's
        // stable type identity in JIR reporting; JavaScript lowering never treats it as a Rust
        // pointer value on its own.
        field_tys: vec![ty, usize_ty, usize_ty],
        passed: vec![0, 1, 2],
        by_ref: false,
        mutable,
        written: false,
        slice: true,
        i64_pair: false,
        tagged: None,
        recursive: None,
    })
}

/// The flat result shape of `ty`, or `None` when it must stay one materialized value.
fn flat_result(table: &TypeTable, ty: TypeId) -> Option<FlatResult> {
    if crate::lower::js::i64_pair::is_i64(table, ty) {
        return Some(FlatResult {
            ty,
            field_tys: vec![ty],
            passed: vec![0],
            i64_pair: true,
            tagged: None,
            recursive: None,
        });
    }
    if let Some(tagged) = tagged_shape(table, ty) {
        let field_tys = tagged.lane_types();
        let passed = (0..field_tys.len() as u32).collect();
        return Some(FlatResult {
            ty,
            field_tys,
            passed,
            i64_pair: false,
            tagged: Some(tagged),
            recursive: None,
        });
    }
    if let Some(recursive) = recursive_shape(table, ty) {
        let field_tys = recursive.lane_types();
        let passed = (0..field_tys.len() as u32).collect();
        return Some(FlatResult {
            ty,
            field_tys,
            passed,
            i64_pair: false,
            tagged: None,
            recursive: Some(recursive),
        });
    }
    let (field_tys, passed) = flat_shape(table, ty)?;
    Some(FlatResult {
        ty,
        field_tys,
        passed,
        i64_pair: false,
        tagged: None,
        recursive: None,
    })
}

/// Whether a leaf tagged consumer returns one scalar JavaScript value.
///
/// `flat_result == None` is not this test: it also means an unsupported aggregate such as
/// `ControlFlow`, which is precisely the tag-to-tag boundary this tranche must leave canonical.
fn scalar_leaf_result(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table
            .get(table.peeled(ty))
            .map(|descriptor| &descriptor.kind),
        Some(
            TypeKind::Bool
                | TypeKind::Char
                | TypeKind::Integer { .. }
                | TypeKind::Float { .. }
                | TypeKind::Str
                | TypeKind::Never
        )
    ) || table.is_primitive_enum(ty)
}

/// Whether an operation in a tagged leaf stays inside scalar computation.
///
/// Rust's `wrapping_*` methods may remain direct calls or intrinsics in JIR even though every
/// operand and result is a primitive scalar. Those calls do not create another product boundary
/// and are safe endpoints for `tag, payload...` lanes. A host/indirect call, drop, or call carrying
/// any non-scalar argument/result remains canonical; this excludes the closure-taking transforms
/// that made broad tagged admission grow FingerprintJS.
fn scalar_leaf_op(table: &TypeTable, func: &Func, op: &crate::jir::repr::Op) -> bool {
    let scalar_signature = |args: &[ValueId]| {
        args.iter().all(|argument| {
            func.ty(*argument)
                .is_some_and(|ty| scalar_leaf_result(table, ty))
        }) && op.result.is_none_or(|result| {
            func.ty(result)
                .is_some_and(|ty| table.is_zst(ty) || scalar_leaf_result(table, ty))
        })
    };
    match &op.kind {
        OpKind::Call {
            callee: Callee::Direct { .. },
            function: None,
            args,
            host: None,
            ..
        }
        | OpKind::Intrinsic { args, .. } => scalar_signature(args),
        OpKind::Call { .. } | OpKind::Host { .. } | OpKind::Drop { .. } => false,
        _ => true,
    }
}

/// The bounded tagged-product shape shared by `Option`, `Result`, and ordinary two-variant enums.
///
/// This is deliberately a product rule, not a second enum ABI. The first lane is the existing
/// JavaScript tag and the rest are the distinct payload properties. Each payload property uses
/// the same recursive fixed-product rule as ordinary physical parameters, so `Option<Pair>` is
/// `tag, left, right` rather than `tag, pair-object`. A key used at different physical trees is
/// refused because one JavaScript property cannot truthfully carry two incompatible lane sets.
fn tagged_shape(table: &TypeTable, ty: TypeId) -> Option<TaggedShape> {
    const MAX_TAGGED_LANES: u32 = 12;

    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized
        || descriptor.layout.zst
        || descriptor.layout.needs_drop
        || descriptor.meta.abi == NominalAbi::C
        || descriptor.meta.boundary != PinnedBoundary::None
        || descriptor.js.transparent.is_some()
        || descriptor.js.unsized_tail.is_some()
        || !matches!(
            descriptor.js.enum_repr,
            Some(EnumRepr::Tag { fieldless: false } | EnumRepr::State)
        )
    {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &descriptor.kind
    else {
        return None;
    };
    if variants.len() != 2 {
        return None;
    }

    let mut keyed: Vec<(FieldKey, TaggedLane)> = Vec::new();
    let mut fields = Vec::new();
    for (variant, descriptor) in variants.iter().enumerate() {
        for (field, descriptor) in descriptor.fields.iter().enumerate() {
            fields.push((variant as u32, field as u32));
            if table.is_zst(descriptor.ty) {
                continue;
            }
            if descriptor.key == FieldKey::Transparent {
                return None;
            }
            let node = product_node(table, descriptor.ty, 1)?;
            if let Some((_, lane)) = keyed.iter_mut().find(|(key, _)| *key == descriptor.key) {
                if lane.node != node {
                    return None;
                }
                lane.present_variants.push(variant as u32);
                continue;
            }
            keyed.push((
                descriptor.key,
                TaggedLane {
                    variant: variant as u32,
                    field: field as u32,
                    present_variants: vec![variant as u32],
                    node,
                },
            ));
            if 1 + keyed
                .iter()
                .map(|(_, lane)| lane.node.fields())
                .sum::<u32>()
                > MAX_TAGGED_LANES
            {
                return None;
            }
        }
    }
    if keyed.is_empty() {
        return None;
    }
    Some(TaggedShape {
        ty,
        lanes: keyed.into_iter().map(|(_, lane)| lane).collect(),
        fields,
    })
}

const MAX_RECURSIVE_DEPTH: usize = 4;
const MAX_RECURSIVE_FIELDS: usize = 8;
const MAX_RECURSIVE_LANES: u32 = 12;

/// A nested fixed product worth expanding at the JavaScript ABI boundary.
///
/// Plain scalar structs keep the older JIR-R flat path. This path is selected only when a nested
/// product or a record-shaped pointer contributes more physical structure than one top-level
/// field, which keeps the common two-scalar case small while admitting iterator state.
fn recursive_shape(table: &TypeTable, ty: TypeId) -> Option<RecursiveShape> {
    let root = product_node(table, ty, 0)?;
    let ProductNode::Product { fields, .. } = &root else {
        return None;
    };
    if root.fields() == 0
        || root.fields() > MAX_RECURSIVE_LANES
        || fields.iter().all(simple_product_field)
    {
        return None;
    }
    let semantic_fields = fields
        .iter()
        .enumerate()
        .filter_map(|(field, node)| (node.fields() > 0).then_some(field as u32))
        .collect();
    Some(RecursiveShape {
        ty,
        root,
        fields: semantic_fields,
    })
}

fn simple_product_field(node: &ProductNode) -> bool {
    match node {
        ProductNode::Zst { .. } => true,
        ProductNode::Leaf { components, .. } => components.as_slice() == [LaneComponent::Value],
        ProductNode::Product { .. } => false,
    }
}

/// The physical tree of one copyable semantic value.
///
/// A thin reference remains one identity-bearing lane. Compiler-owned slot and slice pointer
/// records split into their observable components. Fixed products recurse under a small depth and
/// lane bound; enums, custom-drop values, pinned boundaries, and opaque carriers decline.
fn product_node(table: &TypeTable, ty: TypeId, depth: usize) -> Option<ProductNode> {
    if depth > MAX_RECURSIVE_DEPTH {
        return None;
    }
    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized || descriptor.layout.needs_drop {
        return None;
    }
    if descriptor.layout.zst {
        return Some(ProductNode::Zst { ty });
    }

    if let Some(pointer) = descriptor.js.pointer_form {
        let components = match pointer {
            PointerForm::Thin | PointerForm::Str => vec![LaneComponent::Value],
            PointerForm::Slot => vec![LaneComponent::Buffer, LaneComponent::Offset],
            PointerForm::FatSlice => vec![
                LaneComponent::Buffer,
                LaneComponent::Offset,
                LaneComponent::Length,
            ],
            PointerForm::FatDyn | PointerForm::FatTail => return None,
        };
        return Some(ProductNode::Leaf { ty, components });
    }

    if matches!(
        descriptor.kind,
        TypeKind::Bool
            | TypeKind::Char
            | TypeKind::Integer { .. }
            | TypeKind::Float { .. }
            | TypeKind::Str
            | TypeKind::FnPtr { .. }
    ) || descriptor.js.is_primitive_enum
    {
        return Some(ProductNode::Leaf {
            ty,
            components: vec![LaneComponent::Value],
        });
    }

    if descriptor.meta.abi == NominalAbi::C
        || descriptor.meta.boundary != PinnedBoundary::None
        || descriptor.js.transparent.is_some()
        || descriptor.js.unsized_tail.is_some()
    {
        return None;
    }
    if !matches!(
        descriptor.kind,
        TypeKind::Tuple(_)
            | TypeKind::Array { .. }
            | TypeKind::Closure { .. }
            | TypeKind::Adt {
                form: NominalForm::Struct,
                ..
            }
    ) {
        return None;
    }
    let field_tys = candidates::variant_shape(table, ty, 0)?;
    if field_tys.len() > MAX_RECURSIVE_FIELDS {
        return None;
    }
    let fields = field_tys
        .into_iter()
        .map(|field| product_node(table, field, depth + 1))
        .collect::<Option<Vec<_>>>()?;
    let node = ProductNode::Product { ty, fields };
    (node.fields() <= MAX_RECURSIVE_LANES).then_some(node)
}

/// Declaration-order fields and the non-zero-sized lanes of a fixed aggregate shape.
fn flat_shape(table: &TypeTable, aggregate: TypeId) -> Option<(Vec<TypeId>, Vec<u32>)> {
    // Only the shapes whose fields are the same for every value of the type. An enum's are not:
    // `candidates::shape` answers for variant 0, and handing `Err(3)` to a callee that rebuilds
    // `Ok(3)` is a miscompile — so an enum parameter is refused here rather than relying on the
    // escape analysis's own filter to keep saying no. A closure environment is refused for the
    // same reason a `Copy` of one is: nothing calls a closure body directly enough for it to be
    // worth the exception.
    if !matches!(
        table.get(aggregate).map(|d| &d.kind),
        Some(TypeKind::Tuple(_) | TypeKind::Array { .. })
            | Some(TypeKind::Adt {
                form: NominalForm::Struct,
                ..
            })
    ) {
        return None;
    }
    let field_tys = candidates::shape(table, aggregate, 0)?;
    if !candidates::field_reads_resolve(table, aggregate, 0, field_tys.len()) {
        return None;
    }
    // Scalars only. A field that is itself an object would be *aliased* by the rebuild rather than
    // rebuilt, so a write through it would reach the caller's storage in JavaScript and not in the
    // interpreter's model — a difference the summary in `analysis::param_use` does not track.
    if !candidates::shape_survives_copy(table, &field_tys) {
        return None;
    }
    // A zero-sized field is `undefined` everywhere, so passing it says nothing.
    let passed: Vec<u32> = (0..field_tys.len() as u32)
        .filter(|field| !table.is_zst(field_tys[*field as usize]))
        .collect();
    if passed.is_empty() {
        return None;
    }
    Some((field_tys, passed))
}

/// The reference type of `(pointee, mutable)` this program has, if any.
///
/// The caller's copy-back is a `BorrowView` into the pointee followed by a `StoreThrough`, which
/// is the shape `lower/ssa.rs` builds for `(*p).f = v`, and it needs a type for the view. The type
/// table is sorted by [`TypeId`], so the first match is a deterministic one.
fn reference_ty(table: &TypeTable, pointee: TypeId, mutable: bool) -> Option<TypeId> {
    let of = |want: bool| {
        table
            .descriptors()
            .iter()
            .find(|descriptor| {
                matches!(&descriptor.kind, TypeKind::Reference { pointee: p, mutable: m }
                    if *p == pointee && *m == want)
            })
            .map(|descriptor| descriptor.id)
    };
    of(mutable).or_else(|| of(!mutable))
}

/// The type to give the view a copy-back stores through.
///
/// The real `&mut FieldTy` when the program has one. Otherwise the field's own type, which is a
/// stand-in exactly as `lower/ssa.rs::view_ty`'s is: the view is consumed by the `StoreThrough`
/// right after it, and neither the verifier, the printer nor the interpreter reads its type — the
/// verifier only checks the stored value against the *pointee* of a type that has one, which a
/// non-pointer stand-in does not. `None` when the field is itself a pointer, where a wrong
/// stand-in would be checked and would not hold.
fn store_view_ty(table: &TypeTable, field: TypeId) -> Option<TypeId> {
    reference_ty(table, field, true).or_else(|| {
        match table.get(table.peeled(field)).map(|d| &d.kind) {
            Some(TypeKind::Reference { .. } | TypeKind::RawPointer { .. }) => None,
            _ => Some(field),
        }
    })
}

/// Whether `caller`'s own type table spells the same fields the plan chose.
fn caller_agrees(
    table: &BTreeMap<JsName, LinkItem>,
    caller: &JsName,
    shapes: &[Option<FlatParam>],
) -> bool {
    type_table(table, caller).is_some_and(|types| shapes_agree(types, shapes))
}

/// Whether `caller` can rebuild this result and every use of a result from `callee` is field-only.
///
/// `Some` separates local field reads from compatible direct-call forwards. `None` means a
/// whole-value use keeps the canonical result ABI.
fn caller_accepts_result(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    caller: &JsName,
    callee: ItemId,
    result: &FlatResult,
    potential_params: &BTreeMap<ItemId, Vec<Option<FlatParam>>>,
) -> Option<ResultUse> {
    let types = type_table(table, caller)?;
    if result.i64_pair {
        if !crate::lower::js::i64_pair::is_i64(types, result.ty) {
            return None;
        }
        let func = funcs.get(caller)?.as_ref().ok()?;
        let mut saw_call = false;
        for block in &func.blocks {
            for op in &block.ops {
                let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } = &op.kind
                else {
                    continue;
                };
                if *item != callee {
                    continue;
                }
                saw_call = true;
                if let Some(value) = op.result {
                    let definition = func.values.get(value.0 as usize)?;
                    if definition.ty != result.ty || definition.carrier != Carrier::Baseline {
                        return None;
                    }
                }
            }
        }
        return saw_call.then_some(ResultUse {
            read: true,
            ..ResultUse::default()
        });
    }
    let agrees = match &result.tagged {
        Some(tagged) => tagged_shape(types, result.ty).as_ref() == Some(tagged),
        None if result.recursive.is_some() => {
            recursive_shape(types, result.ty).as_ref() == result.recursive.as_ref()
        }
        None => {
            candidates::shape(types, result.ty, 0).as_deref() == Some(result.field_tys.as_slice())
                && candidates::field_reads_resolve(types, result.ty, 0, result.field_tys.len())
        }
    };
    if !agrees {
        return None;
    }
    let func = funcs.get(caller)?.as_ref().ok()?;
    let loop_depths = crate::analysis::alloc_sites::loop_depths(func);
    let mut use_ = ResultUse::default();
    for block in &func.blocks {
        for op in &block.ops {
            let OpKind::Call {
                callee: Callee::Direct { item, .. },
                host: None,
                ..
            } = &op.kind
            else {
                continue;
            };
            if *item != callee {
                continue;
            }
            let Some(value) = op.result else { continue };
            let definition = func.values.get(value.0 as usize)?;
            if definition.ty != result.ty || definition.carrier != Carrier::Baseline {
                return None;
            }
            let observed = field_only_value(func, value, result, potential_params)?;
            use_.read |= observed.read;
            if !observed.forwards.is_empty() {
                use_.forward_sites += 1;
                use_.hot_forward |= loop_depths
                    .get(block.id.0 as usize)
                    .is_some_and(|depth| *depth > 0);
            }
            use_.forwards.extend(observed.forwards);
        }
    }
    Some(use_)
}

/// Field-sensitive uses of one prospective result-lane value.
#[derive(Default)]
struct ResultUse {
    read: bool,
    forwards: BTreeSet<(ItemId, usize)>,
    forward_sites: u32,
    hot_forward: bool,
}

/// Whether `value` and aliases made only by `Move`/`Copy` are consumed exclusively by field reads
/// or by-value forwards into compatible prospective flat parameters.
fn field_only_value(
    func: &Func,
    value: ValueId,
    result: &FlatResult,
    potential_params: &BTreeMap<ItemId, Vec<Option<FlatParam>>>,
) -> Option<ResultUse> {
    let mut aliases = BTreeSet::from([value]);
    loop {
        let before = aliases.len();
        for block in &func.blocks {
            for op in &block.ops {
                let source = match &op.kind {
                    OpKind::Move(source) | OpKind::Copy(source) => *source,
                    _ => continue,
                };
                if aliases.contains(&source) {
                    aliases.insert(op.result?);
                }
            }
        }
        if aliases.len() == before {
            break;
        }
    }
    if aliases.iter().any(|alias| {
        func.values
            .get(alias.0 as usize)
            .is_none_or(|definition| definition.carrier != Carrier::Baseline)
    }) {
        return None;
    }

    let mut use_ = ResultUse::default();
    for block in &func.blocks {
        for op in &block.ops {
            let used_alias = op
                .kind
                .uses()
                .into_iter()
                .any(|used| aliases.contains(&used));
            if !used_alias {
                continue;
            }
            match &op.kind {
                OpKind::Field {
                    base,
                    variant,
                    field,
                } if aliases.contains(base) && result_accepts_field(result, *variant, *field) => {
                    use_.read = true;
                }
                OpKind::Discriminant(base) if aliases.contains(base) && result.tagged.is_some() => {
                    use_.read = true;
                }
                OpKind::Move(source) | OpKind::Copy(source) if aliases.contains(source) => {}
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    args,
                    host: None,
                    ..
                } => {
                    let mut forwarded = false;
                    for (position, argument) in args.iter().enumerate() {
                        if !aliases.contains(argument) {
                            continue;
                        }
                        let shape = potential_params.get(item)?.get(position)?.as_ref()?;
                        if shape.by_ref
                            || shape.slice
                            || shape.ty != result.ty
                            || shape.field_tys != result.field_tys
                            || shape.passed != result.passed
                            || shape.tagged != result.tagged
                            || shape.recursive != result.recursive
                        {
                            return None;
                        }
                        use_.forwards.insert((*item, position));
                        forwarded = true;
                    }
                    if !forwarded {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        if block
            .term
            .uses()
            .into_iter()
            .any(|used| aliases.contains(&used))
        {
            return None;
        }
    }
    Some(use_)
}

/// Whether a tagged/fixed result lane plan can answer this semantic field projection.
fn result_accepts_field(result: &FlatResult, variant: Option<u32>, field: u32) -> bool {
    match &result.tagged {
        Some(tagged) => tagged.fields.contains(&(variant.unwrap_or(0), field)),
        None => {
            variant.is_none()
                && result.recursive.as_ref().map_or_else(
                    || (field as usize) < result.field_tys.len(),
                    |recursive| recursive.fields.contains(&field),
                )
        }
    }
}

/// Whether every whole-result forward still targets a parameter selected by the parameter SCC
/// fixed point.
fn result_forwards_survive(
    forwards: &BTreeSet<(ItemId, usize)>,
    flat: &BTreeMap<ItemId, Vec<bool>>,
) -> bool {
    forwards.iter().all(|(callee, argument)| {
        flat.get(callee)
            .and_then(|params| params.get(*argument))
            .copied()
            .unwrap_or(false)
    })
}

/// Every returned aggregate still has its baseline carrier before the ABI rewrite.
fn returns_baseline(func: &Func, ty: TypeId) -> bool {
    func.blocks.iter().all(|block| match block.term {
        RTerm::Return(Some(value)) => func.values.get(value.0 as usize).is_some_and(|definition| {
            definition.ty == ty && definition.carrier == Carrier::Baseline
        }),
        RTerm::Return(None) => false,
        _ => true,
    })
}

/// Whether `types` spells the same fields the plan chose, and has a type for every view the
/// copy-back needs.
///
/// Two crates share a [`TypeId`] — it is a hash of the monomorphic type — but not a table, and a
/// caller that cannot resolve a field would print a `Field` op for a field it does not know.
fn shapes_agree(types: &TypeTable, shapes: &[Option<FlatParam>]) -> bool {
    shapes.iter().flatten().all(|shape| {
        if shape.i64_pair {
            return crate::lower::js::i64_pair::is_i64(types, shape.ty);
        }
        if shape.slice {
            return types.get(shape.ty).is_some()
                && shape.field_tys[1..]
                    .iter()
                    .all(|ty| types.get(*ty).is_some());
        }
        if let Some(tagged) = &shape.tagged {
            return tagged_shape(types, shape.ty).as_ref() == Some(tagged);
        }
        if let Some(recursive) = &shape.recursive {
            return recursive_shape(types, shape.ty).as_ref() == Some(recursive);
        }
        candidates::shape(types, shape.ty, 0).as_deref() == Some(shape.field_tys.as_slice())
            && candidates::field_reads_resolve(types, shape.ty, 0, shape.field_tys.len())
            && (!shape.by_ref
                || !(shape.mutable && shape.written)
                || shape
                    .passed
                    .iter()
                    .all(|field| store_view_ty(types, shape.field_tys[*field as usize]).is_some()))
    })
}

/// Closes the per-parameter summaries over the calls between them.
///
/// A parameter starts flat and stays flat unless it escapes or is forwarded to a parameter that is
/// not flat — monotone downwards from an optimistic start, so the fixed point is reached in a
/// bounded number of rounds, and taken in [`ItemId`] order so it is the same one every time.
fn close_over_forwards(candidates: &BTreeMap<ItemId, Candidate>) -> BTreeMap<ItemId, Vec<bool>> {
    let mut flat: BTreeMap<ItemId, Vec<bool>> = candidates
        .iter()
        .map(|(item, candidate)| {
            (
                *item,
                candidate.shapes.iter().map(Option::is_some).collect(),
            )
        })
        .collect();
    loop {
        let mut changed = false;
        for (item, candidate) in candidates {
            for (position, use_) in candidate.uses.iter().enumerate() {
                if !flat[item][position] {
                    continue;
                }
                if candidate.shapes[position]
                    .as_ref()
                    .is_some_and(|shape| shape.i64_pair)
                {
                    continue;
                }
                let keeps = !use_.escapes
                    && use_.forwards.iter().all(|(callee, argument)| {
                        flat.get(callee)
                            .and_then(|params| params.get(*argument))
                            .copied()
                            .unwrap_or(false)
                    });
                if !keeps {
                    flat.get_mut(item).expect("a candidate")[position] = false;
                    changed = true;
                }
            }
        }
        if !changed {
            return flat;
        }
    }
}

/// Rewrites one callee to take its flat parameters' fields, in place.
///
/// The old parameter's [`ValueId`] is kept and *redefined* by the op that rebuilds it — the
/// aggregate itself for a by-value parameter, a borrow of that aggregate for a by-reference one —
/// so not a single use in the body has to be touched.
pub fn rewrite_callee(func: &mut Func, sig: &FlatSig) {
    let mut entry_ops: Vec<Op> = Vec::new();
    let mut params: Vec<ValueId> = Vec::new();
    // The rebuilt aggregate of each flat parameter, for the copy-back below.
    let mut aggregates: BTreeMap<usize, ValueId> = BTreeMap::new();
    for (position, param) in func.params.clone().iter().enumerate() {
        let Some(flat) = sig.params.get(position).and_then(Option::as_ref) else {
            params.push(*param);
            continue;
        };
        if flat.slice || flat.i64_pair || flat.tagged.is_some() || flat.recursive.is_some() {
            // Slice, i64 pair, and tagged lanes are physical lowering choices. Keeping the semantic
            // parameter here lets range/enum analysis continue over ordinary JIR; phase C
            // replaces only the JavaScript declaration and direct-call spelling.
            params.push(*param);
            continue;
        }
        let origin = func.values[param.0 as usize].origin;
        // One value per field of the aggregate, in declaration order: a parameter for a field the
        // caller passes, its own zero-sized constant for one the caller leaves out.
        let fields: Vec<ValueId> = flat
            .field_tys
            .iter()
            .enumerate()
            .map(|(field, ty)| {
                let value = define(func, *ty, origin);
                if flat.passed.contains(&(field as u32)) {
                    params.push(value);
                } else {
                    entry_ops.push(Op {
                        result: Some(value),
                        origin,
                        kind: OpKind::Const(ConstTree::Unit),
                    });
                }
                value
            })
            .collect();
        // By value the parameter *is* the aggregate, so the rebuild defines it directly.
        let aggregate = match flat.by_ref {
            false => *param,
            true => define(func, flat.ty, origin),
        };
        entry_ops.push(Op {
            result: Some(aggregate),
            origin,
            kind: OpKind::Aggregate {
                ty: flat.ty,
                variant: 0,
                active_field: None,
                fields,
            },
        });
        if flat.by_ref {
            entry_ops.push(Op {
                result: Some(*param),
                origin,
                kind: OpKind::BorrowView {
                    root: Root::Value(aggregate),
                    path: Vec::new(),
                    mutable: flat.mutable,
                },
            });
        }
        aggregates.insert(position, aggregate);
    }
    func.params.clone_from(&params);
    func.blocks[0].params = params;
    let ops = std::mem::take(&mut func.blocks[0].ops);
    func.blocks[0].ops = entry_ops.into_iter().chain(ops).collect();

    // Result lanes and mutable-reference out-fields leave at every `Return`. A `Throw` writes no
    // spill state: the caller never reaches the immediately-following reads.
    let outs = sig.out_fields();
    if sig.result.is_some() || !outs.is_empty() {
        for block in 0..func.blocks.len() {
            let returned = match &func.blocks[block].term {
                RTerm::Return(value) => *value,
                _ => continue,
            };
            let origin = func.blocks[block]
                .ops
                .last()
                .map(|op| op.origin)
                .unwrap_or(OriginId(0));
            let result_reads: Vec<ValueId> = sig
                .result
                .as_ref()
                .filter(|result| {
                    !result.i64_pair && result.tagged.is_none() && result.recursive.is_none()
                })
                .into_iter()
                .flat_map(|result| {
                    let base = returned.expect("a flat aggregate result returns a value");
                    result
                        .passed
                        .iter()
                        .map(move |field| (result, base, *field))
                })
                .map(|(result, base, field)| {
                    let value = define(func, result.field_tys[field as usize], origin);
                    func.blocks[block].ops.push(Op {
                        result: Some(value),
                        origin,
                        kind: OpKind::Field {
                            base,
                            variant: None,
                            field,
                        },
                    });
                    value
                })
                .collect();
            let out_reads: Vec<ValueId> = outs
                .iter()
                .map(|out| {
                    let value = define(func, sig.out_ty(*out), origin);
                    let base = aggregates[&out.0];
                    func.blocks[block].ops.push(Op {
                        result: Some(value),
                        origin,
                        kind: OpKind::Field {
                            base,
                            variant: None,
                            field: out.1,
                        },
                    });
                    value
                })
                .collect();
            let mut spilled: Vec<ValueId> = result_reads.iter().skip(1).copied().collect();
            spilled.extend(out_reads.iter().skip(usize::from(sig.returns_out_field)));
            let result_offset = sig
                .result
                .as_ref()
                .filter(|result| {
                    result.i64_pair || result.tagged.is_some() || result.recursive.is_some()
                })
                .map_or(0, |_| sig.result_spills());
            for (index, value) in spilled.into_iter().enumerate() {
                func.blocks[block].ops.push(Op {
                    result: None,
                    origin,
                    kind: OpKind::SpillWrite {
                        index: result_offset + index as u32 + 1,
                        v: value,
                    },
                });
            }
            if let Some(primary) = result_reads
                .first()
                .or_else(|| sig.returns_out_field.then(|| &out_reads[0]))
            {
                func.blocks[block].term = RTerm::Return(Some(*primary));
            }
        }
        if let Some(result) = sig.result.as_ref().filter(|result| {
            !result.i64_pair && result.tagged.is_none() && result.recursive.is_none()
        }) {
            func.sig.result = result.field_tys[result.passed[0] as usize];
        } else if sig.returns_out_field {
            func.sig.result = sig.out_ty(outs[0]);
        }
    }

    // The signature the driver and the verifier read the parameter list off.
    func.sig.abi.parameters = func.params.len() as u16;
    func.sig.arguments = func
        .params
        .iter()
        .map(|param| func.values[param.0 as usize].ty)
        .collect();
}

/// Rewrites every call `func` makes to a flat callee, in place.
///
/// `table` is the *caller's* type table, which is what the copy-back's views are typed out of.
pub fn rewrite_calls(func: &mut Func, plan: &AbiPlan, table: Option<&TypeTable>) {
    for block in 0..func.blocks.len() {
        let ops = std::mem::take(&mut func.blocks[block].ops);
        let mut out: Vec<Op> = Vec::with_capacity(ops.len());
        for mut op in ops {
            let sig = match &op.kind {
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } => plan.flat.get(item).cloned(),
                _ => None,
            };
            let (Some(sig), OpKind::Call { args, .. }) = (sig, &op.kind) else {
                out.push(op);
                continue;
            };
            let origin = op.origin;
            let args = args.clone();
            // Per flat parameter: the aggregate the call reads its fields out of, and — for a
            // `&mut` — the pointer the copy-back stores through.
            let mut flat_args: Vec<ValueId> = Vec::new();
            let mut pointers: BTreeMap<usize, ValueId> = BTreeMap::new();
            for (position, arg) in args.iter().enumerate() {
                let Some(flat) = sig.params.get(position).and_then(Option::as_ref) else {
                    flat_args.push(*arg);
                    continue;
                };
                if flat.slice || flat.i64_pair || flat.tagged.is_some() || flat.recursive.is_some()
                {
                    flat_args.push(*arg);
                    continue;
                }
                // Reading the pointee by value is exactly what a `Deref` is; the *writes* below
                // go back through the pointer rather than through this copy.
                let base = match flat.by_ref {
                    false => *arg,
                    true => {
                        let base = define(func, flat.ty, origin);
                        out.push(Op {
                            result: Some(base),
                            origin,
                            kind: OpKind::Deref(*arg),
                        });
                        base
                    }
                };
                for field in &flat.passed {
                    let ty = flat.field_tys[*field as usize];
                    let value = define(func, ty, origin);
                    out.push(Op {
                        result: Some(value),
                        origin,
                        kind: OpKind::Field {
                            base,
                            variant: None,
                            field: *field,
                        },
                    });
                    flat_args.push(value);
                }
                pointers.insert(position, *arg);
            }
            let outs = sig.out_fields();
            let aggregate_result = op.result;
            let flat_primary = match (&sig.result, aggregate_result) {
                (Some(result), Some(_))
                    if !result.i64_pair
                        && result.tagged.is_none()
                        && result.recursive.is_none() =>
                {
                    let value = define(func, result.field_tys[result.passed[0] as usize], origin);
                    op.result = Some(value);
                    Some(value)
                }
                (Some(result), None)
                    if !result.i64_pair
                        && result.tagged.is_none()
                        && result.recursive.is_none() =>
                {
                    op.result = None;
                    None
                }
                (Some(_), _) | (None, _) => None,
            };
            if sig.result.is_none() && sig.returns_out_field {
                op.result = Some(define(func, sig.out_ty(outs[0]), origin));
            }
            let primary = op.result;
            if let OpKind::Call { args, .. } = &mut op.kind {
                *args = flat_args;
            }
            out.push(op);

            // Rebuild the semantic aggregate result under its original ValueId. Partial escape
            // analysis removes this node because selection proved every alias is field-only.
            if let (Some(result), Some(aggregate), Some(first)) =
                (&sig.result, aggregate_result, flat_primary)
            {
                debug_assert!(
                    !result.i64_pair && result.tagged.is_none() && result.recursive.is_none()
                );
                let mut lane = 0_u32;
                let mut fields = Vec::with_capacity(result.field_tys.len());
                for (field, ty) in result.field_tys.iter().enumerate() {
                    let field = field as u32;
                    let value = if result.passed.contains(&field) {
                        let value = if lane == 0 {
                            first
                        } else {
                            let value = define(func, *ty, origin);
                            out.push(Op {
                                result: Some(value),
                                origin,
                                kind: OpKind::SpillRead(lane),
                            });
                            value
                        };
                        lane += 1;
                        value
                    } else {
                        let value = define(func, *ty, origin);
                        out.push(Op {
                            result: Some(value),
                            origin,
                            kind: OpKind::Const(ConstTree::Unit),
                        });
                        value
                    };
                    fields.push(value);
                }
                out.push(Op {
                    result: Some(aggregate),
                    origin,
                    kind: OpKind::Aggregate {
                        ty: result.ty,
                        variant: 0,
                        active_field: None,
                        fields,
                    },
                });
            }

            // Copy mutable-reference out-fields back through the caller's pointer. Result lanes
            // occupy the first spill registers; an out-field uses the primary return only in the
            // old `fn(..) -> ()` case. `BorrowView`/`StoreThrough` is `lower/ssa.rs`'s shape for
            // `(*p).f = v`; a `SetField` would mutate only the `Deref` copy.
            for (index, out_field) in outs.iter().enumerate() {
                let field_ty = sig.out_ty(*out_field);
                let value = match (index, sig.returns_out_field) {
                    (0, true) => primary.expect("a flat return defines a value"),
                    _ => {
                        let register = sig.result_spills() + index as u32 + 1
                            - u32::from(sig.returns_out_field);
                        let value = define(func, field_ty, origin);
                        out.push(Op {
                            result: Some(value),
                            origin,
                            kind: OpKind::SpillRead(register),
                        });
                        value
                    }
                };
                let view_ty = table
                    .and_then(|table| store_view_ty(table, field_ty))
                    .unwrap_or(field_ty);
                let view = define(func, view_ty, origin);
                out.push(Op {
                    result: Some(view),
                    origin,
                    kind: OpKind::BorrowView {
                        root: Root::Value(pointers[&out_field.0]),
                        path: vec![
                            Proj::Deref,
                            Proj::Field {
                                field: out_field.1,
                                ty: field_ty,
                            },
                        ],
                        mutable: true,
                    },
                });
                out.push(Op {
                    result: None,
                    origin,
                    kind: OpKind::StoreThrough {
                        ptr: view,
                        v: value,
                    },
                });
            }
        }
        func.blocks[block].ops = out;
    }
}

/// Defines a fresh SSA value of type `ty`. The caller owes it exactly one definition site.
fn define(func: &mut Func, ty: TypeId, origin: OriginId) -> ValueId {
    let id = ValueId(func.values.len() as u32);
    func.values.push(ValueDef {
        ty,
        carrier: Carrier::Baseline,
        origin,
    });
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::interns::StrId;
    use crate::jir::repr::{FuncBuilder, Proj};
    use crate::jir::semantic::{
        AbiSignature, CastKind, DefinitionId, DirectItemRef, GlobalReference, HostPayload, LocalId,
        Signature, SpreadArgument,
    };
    use crate::jir::types::{
        FieldDescriptor, FieldKey, JsFacts, LayoutFacts, TypeDescriptor, TypeMeta,
    };
    use crate::jir::{Interns, OriginId};

    const U32: TypeId = TypeId(1);
    const PAIR: TypeId = TypeId(2);
    const REF_PAIR: TypeId = TypeId(3);
    const UNIT: TypeId = TypeId(4);
    const FN_DEF: TypeId = TypeId(5);
    /// `Result<u32, u32>`: two variants, so its fields depend on the value.
    const RESULT: TypeId = TypeId(6);
    /// `(u32, ())`: one field that says nothing.
    const CTX: TypeId = TypeId(7);
    const REF_U32: TypeId = TypeId(8);
    const NO_ORIGIN: OriginId = OriginId(0);

    fn item(local_hash: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash,
            },
            instantiation: u128::from(local_hash),
        }
    }

    fn descriptor(id: TypeId, kind: TypeKind) -> TypeDescriptor {
        let zst = id == UNIT || id == FN_DEF;
        let aggregate = matches!(id, PAIR | RESULT | CTX);
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(if zst { 0 } else { 8 }),
                zst,
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: aggregate,
                needs_clone: aggregate,
                crosses_transparent_union: false,
                transparent: None,
                peeled: id,
                unsized_tail: None,
                enum_repr: match id {
                    RESULT => Some(crate::jir::types::EnumRepr::Tag { fieldless: false }),
                    _ => None,
                },
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: match id {
                    PAIR => (0..2)
                        .map(|index| FieldDescriptor {
                            key: FieldKey::Index(index),
                            ty: U32,
                            offset: u64::from(index) * 4,
                        })
                        .collect(),
                    CTX => vec![
                        FieldDescriptor {
                            key: FieldKey::Index(0),
                            ty: U32,
                            offset: 0,
                        },
                        FieldDescriptor {
                            key: FieldKey::Index(1),
                            ty: UNIT,
                            offset: 4,
                        },
                    ],
                    _ => Vec::new(),
                },
            },
        }
    }

    /// `u32`, `(u32, u32)`, `&mut (u32, u32)`, `()`, and a `fn` item resolving to item 9.
    fn interns() -> Interns {
        let mut interns = Interns::new();
        for descriptor in [
            descriptor(
                U32,
                TypeKind::Integer {
                    signed: false,
                    bits: 32,
                },
            ),
            descriptor(PAIR, TypeKind::Tuple(vec![U32, U32])),
            descriptor(
                REF_PAIR,
                TypeKind::Reference {
                    pointee: PAIR,
                    mutable: true,
                },
            ),
            descriptor(UNIT, TypeKind::Tuple(Vec::new())),
            descriptor(CTX, TypeKind::Tuple(vec![U32, UNIT])),
            descriptor(
                REF_U32,
                TypeKind::Reference {
                    pointee: U32,
                    mutable: true,
                },
            ),
            {
                let variant = |name: u32, ok: bool| crate::jir::types::VariantDescriptor {
                    name: StrId(name),
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Index(0),
                        ty: U32,
                        offset: 4,
                    }],
                    logical_discr: i128::from(ok),
                    physical_tag: None,
                    result_ok: ok,
                    saved_local_keys: Vec::new(),
                };
                descriptor(
                    RESULT,
                    TypeKind::Adt {
                        def: DefinitionId {
                            crate_id: 0,
                            local_hash: 7,
                        },
                        args: vec![U32, U32],
                        form: NominalForm::Enum,
                        variants: vec![variant(0, true), variant(0, false)],
                        niche: None,
                    },
                )
            },
            descriptor(
                FN_DEF,
                TypeKind::FnDef {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 9,
                    },
                    sig: crate::jir::interns::SigId(0),
                    instance: Some(DirectItemRef {
                        item: item(9),
                        link_name: StrId(0),
                    }),
                },
            ),
        ] {
            interns.types.insert(descriptor);
        }
        interns
    }

    fn signature(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        let parameters = arguments.len() as u16;
        Signature {
            arguments,
            result,
            abi: AbiSignature {
                parameters,
                spread: None,
                caller_location: false,
            },
        }
    }

    /// `fn step(p: &mut (u32, u32)) { let d = *p; d.1 = d.0; }`
    fn callee() -> Func {
        let mut builder = FuncBuilder::new(item(1), signature(vec![REF_PAIR], UNIT));
        let p = builder.param(REF_PAIR, NO_ORIGIN);
        let d = builder.op(FuncBuilder::ENTRY, OpKind::Deref(p), PAIR, NO_ORIGIN);
        let high = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: d,
                variant: None,
                field: 0,
            },
            U32,
            NO_ORIGIN,
        );
        builder.effect(
            FuncBuilder::ENTRY,
            OpKind::SetField {
                base: d,
                variant: None,
                field: 1,
                v: high,
            },
            NO_ORIGIN,
        );
        builder.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        builder.finish()
    }

    /// `fn caller(x: u32) -> u32 { let mut p = (x, x); step(&mut p); p.0 }`
    fn caller() -> Func {
        let mut builder = FuncBuilder::new(item(2), signature(vec![U32], U32));
        let x = builder.param(U32, NO_ORIGIN);
        let pair = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: PAIR,
                variant: 0,
                active_field: None,
                fields: vec![x, x],
            },
            PAIR,
            NO_ORIGIN,
        );
        let view = builder.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(pair),
                path: Vec::new(),
                mutable: true,
            },
            REF_PAIR,
            NO_ORIGIN,
        );
        builder.effect(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(1),
                    link_name: StrId(0),
                },
                function: None,
                args: vec![view],
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        let low = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: pair,
                variant: None,
                field: 0,
            },
            U32,
            NO_ORIGIN,
        );
        builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(low)));
        builder.finish()
    }

    /// `fn make_pair(x: u32) -> Pair { Pair(x, x) }`
    fn result_callee() -> Func {
        let mut builder = FuncBuilder::new(item(3), signature(vec![U32], PAIR));
        let x = builder.param(U32, NO_ORIGIN);
        let pair = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: PAIR,
                variant: 0,
                active_field: None,
                fields: vec![x, x],
            },
            PAIR,
            NO_ORIGIN,
        );
        builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(pair)));
        builder.finish()
    }

    /// `fn use_pair(x: u32) -> u32 { make_pair(x).0 }`
    fn result_caller() -> Func {
        let mut builder = FuncBuilder::new(item(4), signature(vec![U32], U32));
        let x = builder.param(U32, NO_ORIGIN);
        let pair = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(3),
                    link_name: StrId(0),
                },
                function: None,
                args: vec![x],
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: None,
            },
            PAIR,
            NO_ORIGIN,
        );
        let left = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: pair,
                variant: None,
                field: 0,
            },
            U32,
            NO_ORIGIN,
        );
        builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(left)));
        builder.finish()
    }

    /// `fn forward_pair(x: u32) -> u32 { consume_pair(make_pair(x)) }`
    fn forwarding_result_caller() -> Func {
        let mut builder = FuncBuilder::new(item(4), signature(vec![U32], U32));
        let x = builder.param(U32, NO_ORIGIN);
        let pair = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(3),
                    link_name: StrId(0),
                },
                function: None,
                args: vec![x],
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: None,
            },
            PAIR,
            NO_ORIGIN,
        );
        let consumed = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(5),
                    link_name: StrId(0),
                },
                function: None,
                args: vec![pair],
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(consumed)));
        builder.finish()
    }

    fn flat_pair() -> FlatParam {
        FlatParam {
            ty: PAIR,
            field_tys: vec![U32, U32],
            passed: vec![0, 1],
            by_ref: true,
            mutable: true,
            written: true,
            slice: false,
            i64_pair: false,
            tagged: None,
            recursive: None,
        }
    }

    fn flat_sig() -> FlatSig {
        FlatSig {
            params: vec![Some(flat_pair())],
            result: None,
            spill: 1,
            returns_out_field: true,
        }
    }

    #[test]
    fn slice_positions_account_for_earlier_product_expansion() {
        let slice = FlatParam {
            ty: REF_PAIR,
            field_tys: vec![REF_PAIR, U32, U32],
            passed: vec![0, 1, 2],
            by_ref: false,
            mutable: false,
            written: false,
            slice: true,
            i64_pair: false,
            tagged: None,
            recursive: None,
        };
        let sig = FlatSig {
            params: vec![Some(flat_pair()), Some(slice), None],
            result: None,
            spill: 0,
            returns_out_field: false,
        };
        assert_eq!(sig.slice_param_positions(), vec![1]);
        assert_eq!(sig.slice_jir_positions(), vec![2]);
    }

    #[test]
    fn a_private_clone_inherits_its_sources_direct_call_retargets() {
        let source = JsName::new("source".to_owned());
        let clone = JsName::new("source$n1".to_owned());
        let target = JsName::new("target$n1".to_owned());
        let mut plan = AbiPlan::default();
        plan.clones.insert(clone.clone(), source.clone());
        plan.call_retargets
            .entry(source)
            .or_default()
            .insert(item(2), target.clone());

        inherit_clone_retargets(&mut plan);

        assert_eq!(plan.call_retargets(&clone).get(&item(2)), Some(&target));
    }

    #[test]
    fn economic_groups_couple_shared_callers_and_subset_the_executable_plan() {
        let one = JsName::new("one".to_owned());
        let two = JsName::new("two".to_owned());
        let three = JsName::new("three".to_owned());
        let shared = JsName::new("shared".to_owned());
        let separate = JsName::new("separate".to_owned());
        let mut plan = AbiPlan::default();
        for (id, name) in [
            (item(1), one.clone()),
            (item(2), two.clone()),
            (item(3), three.clone()),
        ] {
            plan.flat.insert(id, FlatSig::default());
            plan.names.insert(id, name.clone());
            plan.select(&name, &FlatSig::default());
        }
        plan.callers
            .insert(item(1), BTreeSet::from([shared.clone()]));
        plan.callers.insert(item(2), BTreeSet::from([shared]));
        plan.callers.insert(item(3), BTreeSet::from([separate]));

        assert_eq!(
            plan.economic_groups(),
            vec![
                BTreeSet::from([item(1), item(2)]),
                BTreeSet::from([item(3)])
            ]
        );
        let subset = plan.subset(&BTreeSet::from([item(1), item(2)]));
        assert_eq!(
            subset.flat.keys().copied().collect::<Vec<_>>(),
            vec![item(1), item(2)]
        );
        assert_eq!(subset.source_name(item(1)), Some(&one));
        assert!(!subset.touches(&three));
    }

    #[test]
    fn economic_groups_close_transitively_across_a_later_bridge() {
        let mut plan = AbiPlan::default();
        for (id, name) in [(item(1), "one"), (item(2), "two"), (item(3), "three")] {
            let name = JsName::new(name.to_owned());
            plan.flat.insert(id, FlatSig::default());
            plan.names.insert(id, name.clone());
            plan.select(&name, &FlatSig::default());
        }
        let ac = JsName::new("ac".to_owned());
        let bc = JsName::new("bc".to_owned());
        plan.callers.insert(item(1), BTreeSet::from([ac.clone()]));
        plan.callers.insert(item(2), BTreeSet::from([bc.clone()]));
        plan.callers.insert(item(3), BTreeSet::from([ac, bc]));

        assert_eq!(
            plan.economic_groups(),
            vec![BTreeSet::from([item(1), item(2), item(3)])]
        );
    }

    fn flat_result_sig() -> FlatSig {
        FlatSig {
            params: vec![None],
            result: Some(flat_result(&interns().types, PAIR).expect("a pair result")),
            spill: 1,
            returns_out_field: false,
        }
    }

    fn link_item(name: &str, refs: &[&str]) -> LinkItem {
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: refs.iter().map(|r| JsName::new((*r).to_owned())).collect(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: String::new(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: Default::default(),
        }
    }

    fn ineligible_of(op: OpKind) -> BTreeSet<ItemId> {
        let interns = interns();
        let mut scan = RefScan::default();
        refs::scan_op(
            &callee(),
            &Op {
                result: None,
                origin: NO_ORIGIN,
                kind: op,
            },
            Some(&interns.types),
            &mut scan,
        );
        scan.ineligible()
    }

    #[test]
    fn a_reference_to_an_aggregate_is_a_flat_parameter_and_a_scalar_is_not() {
        let interns = interns();
        let flat = flat_param(&interns.types, REF_PAIR, true).expect("a reference to a tuple");
        assert_eq!(flat.field_tys, vec![U32, U32]);
        assert_eq!(flat.passed, vec![0, 1]);
        assert!(flat.by_ref && flat.mutable && flat.written);
        assert_eq!(
            flat_param(&interns.types, PAIR, true).map(|flat| flat.by_ref),
            Some(false)
        );
        assert_eq!(flat_param(&interns.types, U32, true), None);
    }

    #[test]
    fn an_exported_callee_is_refused() {
        assert_eq!(
            refuses(
                Some(&Linkage::exported("step".to_owned())),
                None,
                &callee(),
                false,
                &BTreeSet::new()
            ),
            Some(BoundaryDecline::Exported)
        );
    }

    #[test]
    fn a_spread_or_track_caller_abi_is_refused() {
        let mut spread = callee();
        spread.sig.abi.spread = Some(SpreadArgument {
            local: LocalId(1),
            arity: 2,
        });
        assert_eq!(
            refuses(
                Some(&Linkage::internal()),
                None,
                &spread,
                false,
                &BTreeSet::new()
            ),
            Some(BoundaryDecline::SpreadAbi)
        );
        let mut tracked = callee();
        tracked.sig.abi.caller_location = true;
        assert_eq!(
            refuses(
                Some(&Linkage::internal()),
                None,
                &tracked,
                false,
                &BTreeSet::new()
            ),
            Some(BoundaryDecline::SpreadAbi)
        );
    }

    #[test]
    fn a_chunked_program_refuses_the_whole_plan() {
        assert_eq!(
            refuses(
                Some(&Linkage::internal()),
                None,
                &callee(),
                true,
                &BTreeSet::new()
            ),
            Some(BoundaryDecline::Chunks)
        );
    }

    #[test]
    fn drop_glue_is_refused_because_host_lowerings_name_it_by_type() {
        assert_eq!(
            refuses(
                Some(&Linkage::internal()),
                Some(BodySemantic::DropGlue(PAIR)),
                &callee(),
                false,
                &BTreeSet::new()
            ),
            Some(BoundaryDecline::NonCallReference)
        );
    }

    #[test]
    fn an_item_named_outside_a_direct_call_is_refused() {
        let ineligible = BTreeSet::from([item(1)]);
        assert_eq!(
            refuses(
                Some(&Linkage::internal()),
                None,
                &callee(),
                false,
                &ineligible
            ),
            Some(BoundaryDecline::NonCallReference)
        );
        assert_eq!(
            refuses(
                Some(&Linkage::internal()),
                None,
                &callee(),
                false,
                &BTreeSet::new()
            ),
            None
        );
    }

    #[test]
    fn a_vtable_entry_keeps_its_item_out_of_the_plan() {
        // A vtable has no `Func`, so it is never in `funcs`: its `refs` name the method.
        let table = BTreeMap::from([
            (JsName::new("vt".to_owned()), link_item("vt", &["step"])),
            (JsName::new("step".to_owned()), link_item("step", &[])),
        ]);
        let funcs = BTreeMap::from([(JsName::new("step".to_owned()), Ok(callee()))]);
        let by_name = BTreeMap::from([("step", item(1))]);
        assert_eq!(
            refs::named_by_unlowered(&table, &funcs, &by_name),
            BTreeSet::from([item(1)])
        );
    }

    #[test]
    fn a_function_constant_names_its_item() {
        assert_eq!(
            ineligible_of(OpKind::Const(ConstTree::Aggregate(vec![
                ConstTree::Global {
                    target: GlobalReference::Function {
                        item: item(9),
                        link_name: StrId(0),
                    },
                    offset: 0,
                }
            ]))),
            BTreeSet::from([item(9)])
        );
    }

    #[test]
    fn a_reified_function_pointer_names_its_item() {
        assert_eq!(
            ineligible_of(OpKind::Cast {
                kind: CastKind::PointerCoercion,
                v: ValueId(0),
                from: FN_DEF,
                to: U32,
            }),
            BTreeSet::from([item(9)])
        );
    }

    #[test]
    fn an_intrinsic_fallback_names_its_item() {
        assert_eq!(
            ineligible_of(OpKind::Intrinsic {
                name: StrId(0),
                types: Vec::new(),
                fallback: Some(DirectItemRef {
                    item: item(9),
                    link_name: StrId(0),
                }),
                args: Vec::new(),
            }),
            BTreeSet::from([item(9)])
        );
    }

    #[test]
    fn a_host_call_names_its_callee_and_its_helpers() {
        assert_eq!(
            ineligible_of(OpKind::Call {
                callee: Callee::Direct {
                    item: item(1),
                    link_name: StrId(0),
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: Some(HostPayload::ViewAbi {
                    marker: StrId(0),
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 0,
                    },
                    helpers: vec![(
                        StrId(0),
                        DirectItemRef {
                            item: item(9),
                            link_name: StrId(0),
                        },
                    )],
                }),
            }),
            BTreeSet::from([item(1), item(9)])
        );
    }

    #[test]
    fn a_host_call_names_the_body_of_a_closure_argument() {
        // `lower/js/host.rs::closure_item` resolves the thunk from the argument's own type.
        let interns = interns();
        let mut scan = RefScan::default();
        let mut builder = FuncBuilder::new(item(2), signature(vec![FN_DEF], UNIT));
        let closure = builder.param(FN_DEF, NO_ORIGIN);
        let func = builder.finish();
        refs::scan_op(
            &func,
            &Op {
                result: None,
                origin: NO_ORIGIN,
                kind: OpKind::Call {
                    callee: Callee::Runtime { member: StrId(0) },
                    function: None,
                    args: vec![closure],
                    track_caller: None,
                    semantic: crate::jir::semantic::CallSemantic::Ordinary,
                    host: Some(HostPayload::Map { marker: StrId(0) }),
                },
            },
            Some(&interns.types),
            &mut scan,
        );
        let ineligible = scan.ineligible();
        // The body named by the type's own definition and instantiation, and the item the `FnDef`
        // resolves to.
        assert!(ineligible.contains(&ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: 9,
            },
            instantiation: FN_DEF.0,
        }));
        assert!(ineligible.contains(&item(9)));
    }

    #[test]
    fn a_plain_direct_call_names_nothing() {
        assert!(ineligible_of(OpKind::Call {
            callee: Callee::Direct {
                item: item(1),
                link_name: StrId(0),
            },
            function: None,
            args: Vec::new(),
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        })
        .is_empty());
    }

    #[test]
    fn every_compiler_decline_maps_to_a_physical_terminal_reason() {
        for decline in Decline::catalog() {
            let reason = terminal_reason(decline.code());
            assert!(
                [
                    "not-considered.",
                    "legality.",
                    "boundary.",
                    "cost.",
                    "lower.",
                    "validation.",
                ]
                .iter()
                .any(|prefix| reason.starts_with(prefix)),
                "{} mapped to {reason}",
                decline.code()
            );
        }
        assert_eq!(
            terminal_reason("encode.constant-unreadable"),
            "lower.encode.constant-unreadable"
        );
        assert_eq!(
            terminal_reason("merge.summary-conflict"),
            "validation.merge.summary-conflict"
        );
    }

    #[test]
    fn the_callee_rewrite_takes_fields_and_returns_out_fields() {
        let interns = interns();
        let mut func = callee();
        func.verify(&interns.types).expect("the fixture verifies");
        rewrite_callee(&mut func, &flat_sig());
        func.verify(&interns.types)
            .expect("the rewritten callee verifies");
        assert_eq!(func.params.len(), 2);
        assert_eq!(func.sig.abi.parameters, 2);
        assert_eq!(func.sig.result, U32);
        assert_eq!(func.sig.arguments, vec![U32, U32]);
        assert!(matches!(
            func.blocks[0].ops[0].kind,
            OpKind::Aggregate { ty: PAIR, .. }
        ));
        assert!(matches!(
            func.blocks[0].ops[1].kind,
            OpKind::BorrowView { .. }
        ));
        let spills = func.blocks[0]
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OpKind::SpillWrite { index: 1, .. }))
            .count();
        assert_eq!(spills, 1);
        assert!(matches!(func.blocks[0].term, RTerm::Return(Some(_))));
    }

    #[test]
    fn the_caller_rewrite_passes_fields_and_reads_the_spill_back() {
        let interns = interns();
        let mut plan = AbiPlan {
            registers: 1,
            ..AbiPlan::default()
        };
        plan.flat.insert(item(1), flat_sig());
        let mut func = caller();
        func.verify(&interns.types).expect("the fixture verifies");
        rewrite_calls(&mut func, &plan, Some(&interns.types));
        func.verify(&interns.types)
            .expect("the rewritten caller verifies");
        let call = func.blocks[0]
            .ops
            .iter()
            .find(|op| matches!(op.kind, OpKind::Call { .. }))
            .expect("the call is still there");
        let OpKind::Call { args, .. } = &call.kind else {
            unreachable!()
        };
        assert_eq!(args.len(), 2);
        assert!(call.result.is_some());
        assert_eq!(
            func.blocks[0]
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::SpillRead(1)))
                .count(),
            1
        );
        assert_eq!(
            func.blocks[0]
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::StoreThrough { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn the_callee_rewrite_returns_an_aggregate_as_lanes() {
        let interns = interns();
        let mut func = result_callee();
        func.verify(&interns.types).expect("the fixture verifies");
        rewrite_callee(&mut func, &flat_result_sig());
        func.verify(&interns.types)
            .expect("the rewritten result callee verifies");
        assert_eq!(func.sig.result, U32);
        assert!(matches!(func.blocks[0].term, RTerm::Return(Some(_))));
        assert_eq!(
            func.blocks[0]
                .ops
                .iter()
                .filter(|op| matches!(op.kind, OpKind::SpillWrite { index: 1, .. }))
                .count(),
            1
        );
    }

    #[test]
    fn the_caller_rewrite_rebuilds_a_flat_result_for_sroa() {
        let interns = interns();
        let mut plan = AbiPlan {
            registers: 1,
            ..AbiPlan::default()
        };
        plan.flat.insert(item(3), flat_result_sig());
        let mut func = result_caller();
        let aggregate = func.blocks[0].ops[0].result.expect("the original result");
        rewrite_calls(&mut func, &plan, Some(&interns.types));
        func.verify(&interns.types)
            .expect("the rewritten result caller verifies");
        let call = &func.blocks[0].ops[0];
        assert!(call.result.is_some_and(|value| value != aggregate));
        assert!(func.blocks[0]
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::SpillRead(1))));
        assert!(func.blocks[0].ops.iter().any(|op| {
            op.result == Some(aggregate) && matches!(op.kind, OpKind::Aggregate { ty: PAIR, .. })
        }));
    }

    #[test]
    fn only_field_consumers_admit_a_flat_result() {
        let result = flat_result(&interns().types, PAIR).expect("a pair result");
        let field_only = result_caller();
        let use_ = field_only_value(&field_only, ValueId(1), &result, &BTreeMap::new())
            .expect("field-only");
        assert!(use_.read && use_.forwards.is_empty());

        let mut returned_whole = result_caller();
        returned_whole.sig.result = PAIR;
        returned_whole.blocks[0].term = RTerm::Return(Some(ValueId(1)));
        assert!(field_only_value(&returned_whole, ValueId(1), &result, &BTreeMap::new()).is_none());
    }

    #[test]
    fn a_result_may_forward_whole_only_into_a_selected_product_parameter() {
        let types = &interns().types;
        let result = flat_result(types, PAIR).expect("a pair result");
        let by_value = flat_param(types, PAIR, false).expect("a pair parameter");
        let potential = BTreeMap::from([(item(5), vec![Some(by_value)])]);
        let use_ = field_only_value(&forwarding_result_caller(), ValueId(1), &result, &potential)
            .expect("a compatible direct-call forward");
        assert!(!use_.read);
        assert_eq!(use_.forwards, BTreeSet::from([(item(5), 0)]));

        let selected = BTreeMap::from([(item(5), vec![true])]);
        assert!(result_forwards_survive(&use_.forwards, &selected));
        let declined = BTreeMap::from([(item(5), vec![false])]);
        assert!(!result_forwards_survive(&use_.forwards, &declined));
    }

    #[test]
    fn a_spill_register_is_a_module_scope_let_the_walk_can_follow() {
        let plan = AbiPlan {
            registers: 2,
            spill_names: allocate_spill_names(2, &BTreeMap::new()).expect("two free names"),
            ..AbiPlan::default()
        };
        let items = plan.spill_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name.as_str(), "$r1");
        assert_eq!(items[1].js, "\nlet $r2;\n");
        assert!(items[0].kind.is_binding());
    }

    /// The program may bind `$r1` itself — `#[unsafe(export_name = "$r1")]` is all it takes, and
    /// a `link_dedup` winner keeps whichever name the program gave it. Overwriting the item with a
    /// `let` would delete it, so the register steps aside instead.
    #[test]
    fn a_register_steps_past_a_name_the_program_already_uses() {
        let mut table = BTreeMap::new();
        table.insert(JsName::new("$r1".to_owned()), link_item("$r1", &[]));
        // Not defined by any item, only *mentioned* by one — an inline `js!` block naming a
        // global, say. A module-scope `let` of that name would capture the mention.
        table.insert(
            JsName::new("user".to_owned()),
            link_item("user", &["$r2", "$r1$1"]),
        );
        let names = allocate_spill_names(2, &table).expect("names are available");
        assert_eq!(
            names.iter().map(JsName::as_str).collect::<Vec<_>>(),
            ["$r1$2", "$r2$1"]
        );
        // A `$h`-shaped name is one `link::stub_missing` would synthesize a throwing body for, so
        // a register must never be spelled like one. None of the candidates can be.
        assert!(names.iter().all(|name| !name.as_str().contains("$h")));
    }

    /// The belt-and-braces path: nowhere left to put a register at all.
    #[test]
    fn a_register_with_no_name_left_refuses_the_plan() {
        let mut table = BTreeMap::new();
        for name in
            std::iter::once("$r1".to_owned()).chain((1..64).map(|suffix| format!("$r1${suffix}")))
        {
            table.insert(JsName::new(name.clone()), link_item(&name, &[]));
        }
        assert!(allocate_spill_names(1, &table).is_none());
        // Which is a decline of the whole plan, recorded per callee like any other.
        assert_eq!(
            Decline::Boundary(BoundaryDecline::GeneratedNameCollision).code(),
            "boundary.generated-name-collision"
        );
    }

    #[test]
    fn the_sidecar_block_names_every_flat_and_declined_item() {
        let mut plan = AbiPlan {
            registers: 1,
            ..AbiPlan::default()
        };
        plan.flat.insert(item(1), flat_sig());
        plan.names.insert(item(1), JsName::new("step".to_owned()));
        plan.declined.insert(
            JsName::new("keep".to_owned()),
            Decline::Boundary(BoundaryDecline::ParamEscapes),
        );
        assert_eq!(
            plan.json(true).expect("a plan was made"),
            "{\"applied\":true,\"registers\":1,\"flat\":[{\"item\":\"step\",\"params\":[2],\"result\":0,\"spill\":1,\"canonical_wrapper\":false}],\
             \"declined\":[{\"item\":\"keep\",\"reason\":\"boundary.param-escapes\"}]}"
        );
        assert!(AbiPlan::default().json(true).is_none());
    }

    #[test]
    fn a_small_enum_uses_the_common_tagged_product_shape() {
        let interns = interns();
        let param = flat_param(&interns.types, RESULT, true).expect("a tagged result parameter");
        assert_eq!(param.fields(), 2);
        assert_eq!(param.field_tys, vec![U32]);
        assert_eq!(
            param.tagged.as_ref().map(|shape| shape.fields.as_slice()),
            Some(&[(0, 0), (1, 0)][..])
        );
        let result = flat_result(&interns.types, RESULT).expect("a tagged result value");
        assert_eq!(result.fields(), 2);
        assert_eq!(result.tagged, param.tagged);
        assert!(scalar_leaf_result(&interns.types, U32));
        assert!(!scalar_leaf_result(&interns.types, RESULT));
        // The zero-sized and the non-aggregate shapes have nothing to flatten either.
        assert_eq!(flat_param(&interns.types, UNIT, true), None);
        assert_eq!(flat_param(&interns.types, FN_DEF, true), None);
    }

    #[test]
    fn a_zero_sized_field_is_rebuilt_rather_than_passed() {
        let interns = interns();
        let flat = flat_param(&interns.types, CTX, true).expect("a tuple with a unit field");
        assert_eq!(flat.field_tys, vec![U32, UNIT]);
        assert_eq!(flat.passed, vec![0]);
        assert_eq!(flat.fields(), 1);
    }

    #[test]
    fn an_unwritten_mutable_reference_needs_no_copy_back() {
        let interns = interns();
        let read_only = flat_param(&interns.types, REF_PAIR, false).expect("a reference");
        assert!(read_only.mutable && !read_only.written);
        let sig = FlatSig {
            params: vec![Some(read_only)],
            result: None,
            spill: 0,
            returns_out_field: false,
        };
        assert!(sig.out_fields().is_empty());
        assert_eq!(flat_sig().out_fields(), vec![(0, 0), (0, 1)]);
    }

    #[test]
    fn param_use_reports_whether_the_body_writes_through_the_reference() {
        use crate::analysis::param_use::{self, ParamKind};
        let interns = interns();
        let kinds = [Some(ParamKind::ByRef)];
        // `callee` writes `(*p).1`.
        let written = param_use::summarize(&callee(), &interns.types, &kinds);
        assert!(!written[0].escapes && written[0].written);
        // A body that only reads a field does not.
        let mut builder = FuncBuilder::new(item(3), signature(vec![REF_PAIR], U32));
        let p = builder.param(REF_PAIR, NO_ORIGIN);
        let d = builder.op(FuncBuilder::ENTRY, OpKind::Deref(p), PAIR, NO_ORIGIN);
        let high = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Field {
                base: d,
                variant: None,
                field: 0,
            },
            U32,
            NO_ORIGIN,
        );
        builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(high)));
        let read = param_use::summarize(&builder.finish(), &interns.types, &kinds);
        assert!(!read[0].escapes && !read[0].written);
    }

    #[test]
    fn a_tracked_caller_pins_its_callee_and_reports_its_own_reason() {
        // The reason is the plan's, and it is not the callee's own ABI that is at fault.
        assert_eq!(
            BoundaryDecline::TrackedCaller.code(),
            "boundary.tracked-caller"
        );
        let mut tracked = caller();
        tracked.sig.abi.caller_location = true;
        // The scan records the callee of every direct call a `#[track_caller]` body makes.
        let mut pinned = BTreeSet::new();
        for block in &tracked.blocks {
            for op in &block.ops {
                if let OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } = &op.kind
                {
                    if tracked.sig.abi.caller_location {
                        pinned.insert(*item);
                    }
                }
            }
        }
        assert_eq!(pinned, BTreeSet::from([item(1)]));
    }

    #[test]
    fn a_caller_that_cannot_spell_the_fields_does_not_agree() {
        let interns = interns();
        let shapes = [Some(flat_pair())];
        assert!(shapes_agree(&interns.types, &shapes));
        // A table that does not know the aggregate at all cannot spell its fields.
        let mut other = crate::jir::types::TypeTable::new();
        other.insert(descriptor(
            U32,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
        ));
        assert!(!shapes_agree(&other, &shapes));
        // And one with no `&mut u32` still agrees: the field's own type stands in for the view.
        let mut no_reference = crate::jir::types::TypeTable::new();
        no_reference.insert(descriptor(
            U32,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
        ));
        no_reference.insert(descriptor(PAIR, TypeKind::Tuple(vec![U32, U32])));
        assert!(shapes_agree(&no_reference, &shapes));
        assert_eq!(store_view_ty(&interns.types, U32), Some(REF_U32));
        assert_eq!(store_view_ty(&no_reference, U32), Some(U32));
        assert_eq!(store_view_ty(&interns.types, REF_PAIR), None);
    }

    #[test]
    fn a_parameter_forwarded_to_one_that_is_not_flat_is_not_flat_either() {
        let escaping = ParamUse {
            escapes: true,
            ..ParamUse::default()
        };
        let forwards = |to: ItemId| ParamUse {
            forwards: BTreeSet::from([(to, 0)]),
            ..ParamUse::default()
        };
        let candidate = |name: &str, use_: ParamUse| Candidate {
            name: JsName::new(name.to_owned()),
            returns_zst: true,
            shapes: vec![Some(flat_pair())],
            result: None,
            result_forwards: BTreeSet::new(),
            result_forward_sites: 0,
            result_hot_forward: false,
            tagged_leaf_consumer: false,
            uses: vec![use_],
            canonical: false,
        };
        // 3 -> 2 -> 1, and 1's parameter escapes: none of the three is flat.
        let candidates = BTreeMap::from([
            (item(1), candidate("one", escaping)),
            (item(2), candidate("two", forwards(item(1)))),
            (item(3), candidate("three", forwards(item(2)))),
        ]);
        let flat = close_over_forwards(&candidates);
        assert_eq!(flat[&item(1)], vec![false]);
        assert_eq!(flat[&item(2)], vec![false]);
        assert_eq!(flat[&item(3)], vec![false]);
        // A forward to a parameter that stays flat keeps its caller flat.
        let candidates = BTreeMap::from([
            (item(1), candidate("one", ParamUse::default())),
            (item(2), candidate("two", forwards(item(1)))),
        ]);
        let flat = close_over_forwards(&candidates);
        assert_eq!(flat[&item(1)], vec![true]);
        assert_eq!(flat[&item(2)], vec![true]);
        // A forward to an item that is not a candidate at all is not flat.
        let candidates = BTreeMap::from([(item(2), candidate("two", forwards(item(9))))]);
        assert_eq!(close_over_forwards(&candidates)[&item(2)], vec![false]);
    }

    #[test]
    fn tagged_admission_requires_a_result_forward_into_a_leaf_scalar_consumer() {
        let interns = interns();
        let param = flat_param(&interns.types, RESULT, false).expect("a tagged parameter");
        let result = flat_result(&interns.types, RESULT).expect("a tagged result");
        let candidate = |name: &str,
                         result: Option<FlatResult>,
                         forwards: BTreeSet<(ItemId, usize)>,
                         use_: ParamUse| {
            let forward_sites = if forwards.is_empty() { 0 } else { 2 };
            Candidate {
                name: JsName::new(name.to_owned()),
                returns_zst: false,
                shapes: vec![Some(param.clone())],
                result,
                result_forwards: forwards,
                result_forward_sites: forward_sites,
                result_hot_forward: false,
                tagged_leaf_consumer: name == "consumer",
                uses: vec![use_],
                canonical: false,
            }
        };
        let mut candidates = BTreeMap::from([
            (
                item(1),
                candidate(
                    "producer",
                    Some(result.clone()),
                    BTreeSet::from([(item(2), 0)]),
                    ParamUse::default(),
                ),
            ),
            (
                item(2),
                candidate(
                    "middle",
                    None,
                    BTreeSet::new(),
                    ParamUse {
                        forwards: BTreeSet::from([(item(3), 0)]),
                        ..ParamUse::default()
                    },
                ),
            ),
            (
                item(3),
                candidate("consumer", None, BTreeSet::new(), ParamUse::default()),
            ),
            (
                item(4),
                candidate(
                    "isolated",
                    Some(result),
                    BTreeSet::new(),
                    ParamUse::default(),
                ),
            ),
        ]);
        let mut cold_single = candidate(
            "cold_single",
            Some(flat_result(&interns.types, RESULT).expect("a tagged result")),
            BTreeSet::from([(item(3), 0)]),
            ParamUse::default(),
        );
        cold_single.result_forward_sites = 1;
        candidates.insert(item(5), cold_single);

        let chains = tagged_chain_members(&candidates);
        assert_eq!(chains.results, BTreeSet::from([item(1)]));
        assert_eq!(chains.params, BTreeSet::from([(item(2), 0), (item(3), 0)]));
        assert!(!chains.results.contains(&item(4)));
        assert!(!chains.results.contains(&item(5)));
        assert!(!chains.params.contains(&(item(4), 0)));
    }

    #[test]
    fn a_retry_keeps_what_the_plan_refused() {
        let mut plan = AbiPlan {
            registers: 1,
            ..AbiPlan::default()
        };
        plan.flat.insert(item(1), flat_sig());
        plan.names.insert(item(1), JsName::new("step".to_owned()));
        plan.touched.insert(JsName::new("step".to_owned()));
        plan.declined.insert(
            JsName::new("keep".to_owned()),
            Decline::Boundary(BoundaryDecline::ParamEscapes),
        );
        let emptied = plan.emptied();
        assert!(emptied.is_empty());
        assert!(!emptied.touches(&JsName::new("step".to_owned())));
        assert!(emptied.spill_items().is_empty());
        assert_eq!(
            emptied.json(true).expect("the refusals survive"),
            "{\"applied\":true,\"registers\":0,\"flat\":[],\"declined\":[{\"item\":\"keep\",\
             \"reason\":\"boundary.param-escapes\"}]}"
        );
    }

    #[test]
    fn the_caller_stores_back_through_the_pointer_and_not_through_the_deref() {
        // Under JIR-R's value semantics a `Deref` yields a copy, so a `SetField` on it would not
        // reach the caller's storage; the interpreter is what says so.
        let interns = interns();
        let mut plan = AbiPlan {
            registers: 1,
            ..AbiPlan::default()
        };
        plan.flat.insert(item(1), flat_sig());
        let mut func = caller();
        rewrite_calls(&mut func, &plan, Some(&interns.types));
        func.verify(&interns.types).expect("verifies");
        assert!(!func.blocks[0]
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::SetField { .. })));
        let stores = func.blocks[0]
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OpKind::StoreThrough { .. }))
            .count();
        assert_eq!(stores, 2);
        assert!(func.blocks[0].ops.iter().any(|op| matches!(
            &op.kind,
            OpKind::BorrowView { path, .. }
                if matches!(path.as_slice(), [Proj::Deref, Proj::Field { field: 1, .. }])
        )));
    }
}
