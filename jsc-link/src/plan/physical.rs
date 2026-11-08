//! Physical choices between optimized JIR and JavaScript lowering.
//!
//! Semantic JIR deliberately does not bake in whether a Rust value travels as one JavaScript
//! value, several scalar lanes, addressable storage, or a native carrier. This module is the
//! common vocabulary for those choices. [`PhysicalPlan::of`] derives the flat ABI and carrier
//! choices for the lane sidecar and proves value-local exact numeric storage. Numeric storage is
//! observational by default and executable only under its explicit link-time opt-in.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::item::{JsName, LinkItem, Linkage};
use crate::jir::repr::{Carrier, Func, OpKind, RTerm, Root, ValueId};
use crate::jir::semantic::{CallSemantic, Callee, CastKind, ConstTree, StandardOperation};
use crate::jir::types::{
    AllocatorIdentity, PinnedBoundary, RepresentationCandidate, TypeId, TypeKind, TypeTable,
};
use crate::lane_report::push_string;
use crate::plan::abi_flat::{
    AbiDecision, AbiPlan, FlatParam, FlatResult, LaneComponent, ProductNode,
};
use crate::plan::physical_cost::{Evaluation, ProgramMetrics};

/// One scalar JavaScript lane.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ScalarKind {
    Bool,
    Char,
    Signed(u32),
    Unsigned(u32),
    Float(u32),
    Reference,
    HostString,
    Opaque,
}

/// One member of a physical product.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LaneKind {
    Scalar(ScalarKind),
    Tag,
    Buffer,
    Value,
}

/// The JavaScript shape in which one semantic Rust value crosses a boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValueShape {
    Scalar(ScalarKind),
    Product(Vec<LaneKind>),
    Materialized(Carrier),
}

/// Whether a canonical materialized entry point remains at this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WrapperPolicy {
    None,
    Canonical,
}

/// One function's selected call shape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallAbi {
    pub item: JsName,
    pub params: Vec<ValueShape>,
    pub result: ValueShape,
    pub wrapper: WrapperPolicy,
}

/// Where a live value resides after physicalization.
#[allow(dead_code)] // Several storage policies remain modeled but deliberately unavailable.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum StoragePlan {
    Virtual,
    ScalarLocals,
    AddressableCells,
    JsArray,
    TypedArray,
    JsObject,
    Baseline,
}

/// How a value's cleanup is represented.
#[allow(dead_code)] // Modern disposal forms remain explicit, unavailable policy choices.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DropPlan {
    Elided,
    Direct,
    TryFinally,
    JsDispose,
    JsAsyncDispose,
    DisposableStack,
    Baseline,
}

/// One exact/reserved numeric-storage opportunity recorded before executable selection.
#[derive(Clone, Debug, Eq, PartialEq)]
struct StorageDecision {
    item: JsName,
    value: ValueId,
    element: String,
    typed_array: Option<&'static str>,
    constant_initial_capacity: Option<u64>,
    proven_maximum_length: Option<u64>,
    proven_exact_final_length: Option<u64>,
    selected: bool,
    reason: Option<&'static str>,
}

/// One selected value-local typed backing store. The semantic carrier remains `NativeVec`;
/// this record only changes its private `buf` storage and the calls that grow it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TypedStorage {
    pub constructor: &'static str,
    pub allocation_length: u64,
}

/// A conservative straight-line proof of one vector's logical element count.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LengthProof {
    current: Option<u64>,
    maximum: Option<u64>,
}

#[derive(Debug, Default)]
struct VectorAnalysis {
    proofs: BTreeMap<ValueId, LengthProof>,
    aliases: BTreeMap<ValueId, ValueId>,
}

impl LengthProof {
    const EMPTY: Self = Self {
        current: Some(0),
        maximum: Some(0),
    };

    fn forget(&mut self) {
        self.current = None;
        self.maximum = None;
    }

    fn push(&mut self) {
        let Some(next) = self.current.and_then(|length| length.checked_add(1)) else {
            self.forget();
            return;
        };
        self.current = Some(next);
        self.maximum = self.maximum.map(|maximum| maximum.max(next));
    }

    fn clear(&mut self) {
        if self.current.is_some() {
            self.current = Some(0);
        }
    }

    fn remove(&mut self) {
        let Some(next) = self.current.and_then(|length| length.checked_sub(1)) else {
            self.forget();
            return;
        };
        self.current = Some(next);
    }
}

/// One value whose cleanup is not elided.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DropDecision {
    item: JsName,
    subject: String,
    plan: DropPlan,
    reason: &'static str,
}

/// The physical plan for the current whole program.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhysicalPlan {
    pub(crate) ready_futures: BTreeSet<TypeId>,
    calls: u32,
    selected_calls: Vec<CallAbi>,
    decisions: Vec<AbiDecision>,
    storage: BTreeMap<StoragePlan, u64>,
    drop: BTreeMap<DropPlan, u64>,
    typed_storage_enabled: bool,
    storage_decisions: Vec<StorageDecision>,
    typed_values: BTreeMap<JsName, BTreeMap<ValueId, TypedStorage>>,
    drop_decisions: Vec<DropDecision>,
    economics: Option<Evaluation>,
}

impl PhysicalPlan {
    pub fn with_economics(mut self, economics: Option<Evaluation>) -> PhysicalPlan {
        self.economics = economics;
        self
    }

    /// Derives the physical choices the existing planners have already made.
    pub fn of(
        table: &BTreeMap<JsName, LinkItem>,
        funcs: &BTreeMap<JsName, Result<Func, crate::jir::decline::Decline>>,
        abi: &AbiPlan,
        select_typed_storage: bool,
    ) -> PhysicalPlan {
        let mut plan = PhysicalPlan {
            decisions: abi.decisions().cloned().collect(),
            typed_storage_enabled: select_typed_storage,
            ..PhysicalPlan::default()
        };
        for (name, entry) in funcs {
            let Ok(func) = entry else { continue };
            let Some(types) = type_table(table, name) else {
                continue;
            };
            plan.calls += 1;
            let (decisions, typed_values) =
                storage_decisions(name, func, types, abi, select_typed_storage);
            plan.storage_decisions.extend(decisions);
            if !typed_values.is_empty() {
                plan.typed_values.insert(name.clone(), typed_values);
            }
            for (index, value) in func.values.iter().enumerate() {
                *plan
                    .storage
                    .entry(storage_plan(types, value.ty, value.carrier))
                    .or_default() += 1;
                let cleanup = drop_plan(types, value.ty, value.carrier);
                *plan.drop.entry(cleanup).or_default() += 1;
                if cleanup != DropPlan::Elided {
                    plan.drop_decisions.push(DropDecision {
                        item: name.clone(),
                        subject: format!("v{index}"),
                        plan: cleanup,
                        reason: drop_reason(cleanup, value.carrier),
                    });
                }
            }
            for (index, cell) in func.cells.iter().enumerate() {
                let storage = if cell.boxed {
                    StoragePlan::AddressableCells
                } else {
                    StoragePlan::ScalarLocals
                };
                *plan.storage.entry(storage).or_default() += 1;
                let cleanup = drop_plan(types, cell.ty, Carrier::Baseline);
                *plan.drop.entry(cleanup).or_default() += 1;
                if cleanup != DropPlan::Elided {
                    plan.drop_decisions.push(DropDecision {
                        item: name.clone(),
                        subject: format!("c{index}"),
                        plan: cleanup,
                        reason: drop_reason(cleanup, Carrier::Baseline),
                    });
                }
            }

            let flat = abi.target_signature(name, func.item);
            let params: Vec<ValueShape> = func
                .params
                .iter()
                .enumerate()
                .map(|(position, value)| {
                    flat.and_then(|sig| sig.params.get(position))
                        .and_then(Option::as_ref)
                        .map(|param| param_shape(types, param))
                        .unwrap_or_else(|| {
                            let value = func.values[value.0 as usize];
                            value_shape(types, value.ty, value.carrier)
                        })
                })
                .collect();
            let result = flat
                .and_then(|sig| sig.result.as_ref())
                .map(|result| result_shape(types, result))
                .unwrap_or_else(|| value_shape(types, func.sig.result, Carrier::Baseline));
            let selected = flat.is_some()
                || func
                    .params
                    .iter()
                    .any(|value| func.values[value.0 as usize].carrier != Carrier::Baseline);
            if selected {
                let wrapper = if abi.has_canonical_wrapper(name)
                    || table
                        .get(name)
                        .is_some_and(|item| item.linkage != Linkage::internal())
                {
                    WrapperPolicy::Canonical
                } else {
                    WrapperPolicy::None
                };
                plan.selected_calls.push(CallAbi {
                    item: name.clone(),
                    params,
                    result,
                    wrapper,
                });
            }
        }
        plan.selected_calls
            .sort_by(|left, right| left.item.as_str().cmp(right.item.as_str()));
        plan.storage_decisions.sort_by(|left, right| {
            (left.item.as_str(), left.value.0).cmp(&(right.item.as_str(), right.value.0))
        });
        plan.drop_decisions.sort_by(|left, right| {
            (left.item.as_str(), left.subject.as_str())
                .cmp(&(right.item.as_str(), right.subject.as_str()))
        });
        plan
    }

    /// The exact typed backing store selected for `value` in `item`, including references and
    /// moves proven to alias the constructor result.
    pub(crate) fn typed_storage(&self, item: &JsName, value: ValueId) -> Option<TypedStorage> {
        self.typed_values
            .get(item)
            .and_then(|values| values.get(&value))
            .copied()
    }

    /// The schema-14 `physical` block.
    pub fn json(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\"mode\":\"exact-compiler-link-artifact\",\"calls\":{}",
            self.calls
        );
        out.push_str(",\"profile\":");
        match &self.economics {
            Some(economics) => push_string(&mut out, economics.profile),
            None => out.push_str("null"),
        }
        out.push_str(",\"canonical\":");
        metrics_option_json(
            &mut out,
            self.economics
                .as_ref()
                .and_then(|economics| economics.before),
        );
        out.push_str(",\"planned\":");
        metrics_option_json(
            &mut out,
            self.economics
                .as_ref()
                .and_then(|economics| economics.after),
        );
        out.push_str(",\"validated\":");
        metrics_option_json(
            &mut out,
            self.economics
                .as_ref()
                .and_then(|economics| economics.validated),
        );
        out.push_str(",\"final_emitted\":");
        metrics_option_json(
            &mut out,
            self.economics.as_ref().and_then(Evaluation::final_emitted),
        );
        out.push_str(",\"cache_hits\":");
        let _ = write!(
            out,
            "{}",
            self.economics
                .as_ref()
                .map_or(0, |economics| economics.cache_hits)
        );
        out.push_str(",\"selected_calls\":[");
        for (index, call) in self.selected_calls.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, call.item.as_str());
            out.push_str(",\"params\":[");
            for (position, param) in call.params.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                shape_json(&mut out, param);
            }
            out.push_str("],\"result\":");
            shape_json(&mut out, &call.result);
            out.push_str(",\"wrapper\":");
            push_string(&mut out, wrapper_name(call.wrapper));
            out.push('}');
        }
        out.push_str("],\"decisions\":[");
        for (index, decision) in self.decisions.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, decision.item.as_str());
            out.push_str(",\"shape\":");
            match &decision.shape {
                Some(shape) => push_string(&mut out, shape),
                None => out.push_str("null"),
            }
            out.push_str(",\"profile\":");
            match &self.economics {
                Some(economics) => push_string(&mut out, economics.profile),
                None => out.push_str("null"),
            }
            let _ = write!(out, ",\"selected\":{}", decision.selected);
            out.push_str(",\"reason\":");
            match &decision.reason {
                Some(reason) => push_string(&mut out, reason),
                None => out.push_str("null"),
            }
            let decision_economics = self
                .economics
                .as_ref()
                .and_then(|economics| economics.decisions.get(&decision.item));
            out.push_str(",\"before\":");
            metrics_option_json(
                &mut out,
                decision_economics
                    .and_then(|economics| economics.before)
                    .or_else(|| {
                        self.economics
                            .as_ref()
                            .and_then(|economics| economics.before)
                    }),
            );
            out.push_str(",\"after\":");
            metrics_option_json(
                &mut out,
                decision_economics
                    .and_then(|economics| economics.after)
                    .or_else(|| {
                        self.economics
                            .as_ref()
                            .and_then(|economics| economics.after)
                    }),
            );
            out.push_str(",\"hotness_source\":");
            match decision_economics
                .and_then(|economics| economics.hotness_source)
                .or_else(|| {
                    self.economics
                        .as_ref()
                        .and_then(|economics| economics.hotness_source)
                }) {
                Some(source) => push_string(&mut out, source),
                None => out.push_str("null"),
            }
            out.push_str(",\"budget_consumed_bps\":");
            let _ = write!(
                out,
                "{}",
                decision_economics.map_or_else(
                    || self
                        .economics
                        .as_ref()
                        .map_or(0, |economics| economics.budget_consumed_bps),
                    |economics| economics.budget_consumed_bps,
                )
            );
            out.push_str(",\"cross_crate\":{\"summary_available\":");
            let evidence = &decision.cross_crate;
            let _ = write!(
                out,
                "{},\"summary_validated\":{},\"body_decoded\":{},\"caller_count\":{},\
                 \"crosses_crate\":{}",
                evidence.summary_available,
                evidence.summary_validated,
                evidence.body_decoded,
                evidence.caller_count,
                evidence.crosses_crate
            );
            out.push_str(",\"producer_crate\":");
            match &evidence.producer_crate {
                Some(producer) => push_string(&mut out, producer),
                None => out.push_str("null"),
            }
            out.push_str(",\"caller_crates\":[");
            for (index, caller) in evidence.caller_crates.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                push_string(&mut out, caller);
            }
            out.push_str("],\"selected\":");
            let _ = write!(out, "{}", decision.selected);
            out.push_str(",\"reason\":");
            match &decision.reason {
                Some(reason) => push_string(&mut out, reason),
                None => out.push_str("null"),
            }
            out.push('}');
            out.push('}');
        }
        out.push_str("],\"storage\":[");
        counts_json(&mut out, &self.storage, storage_name);
        out.push_str("],\"drop\":[");
        counts_json(&mut out, &self.drop, drop_name);
        out.push_str("],\"storage_mode\":");
        push_string(
            &mut out,
            if self.typed_storage_enabled {
                "exact-opt-in"
            } else {
                "report-only"
            },
        );
        out.push_str(",\"storage_decisions\":[");
        for (index, decision) in self.storage_decisions.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, decision.item.as_str());
            let _ = write!(out, ",\"value\":{},\"element\":", decision.value.0);
            push_string(&mut out, &decision.element);
            out.push_str(",\"storage\":\"typed-array\",\"typed_array\":");
            match decision.typed_array {
                Some(typed_array) => push_string(&mut out, typed_array),
                None => out.push_str("null"),
            }
            out.push_str(",\"constant_initial_capacity\":");
            match decision.constant_initial_capacity {
                Some(capacity) => {
                    let _ = write!(out, "{capacity}");
                }
                None => out.push_str("null"),
            }
            out.push_str(",\"proven_maximum_length\":");
            match decision.proven_maximum_length {
                Some(length) => {
                    let _ = write!(out, "{length}");
                }
                None => out.push_str("null"),
            }
            out.push_str(",\"proven_exact_final_length\":");
            match decision.proven_exact_final_length {
                Some(length) => {
                    let _ = write!(out, "{length}");
                }
                None => out.push_str("null"),
            }
            let _ = write!(out, ",\"selected\":{}", decision.selected);
            out.push_str(",\"reason\":");
            match decision.reason {
                Some(reason) => push_string(&mut out, reason),
                None => out.push_str("null"),
            }
            out.push('}');
        }
        out.push_str("],\"cleanup_mode\":\"existing-executable\",\"drop_decisions\":[");
        for (index, decision) in self.drop_decisions.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"item\":");
            push_string(&mut out, decision.item.as_str());
            out.push_str(",\"subject\":");
            push_string(&mut out, &decision.subject);
            out.push_str(",\"plan\":");
            push_string(&mut out, drop_name(decision.plan));
            out.push_str(",\"reason\":");
            push_string(&mut out, decision.reason);
            out.push('}');
        }
        out.push_str("]}");
        out
    }
}

fn storage_decisions(
    name: &JsName,
    func: &Func,
    types: &TypeTable,
    abi: &AbiPlan,
    select_typed_storage: bool,
) -> (Vec<StorageDecision>, BTreeMap<ValueId, TypedStorage>) {
    let mut decisions = Vec::new();
    let mut typed_values = BTreeMap::new();
    let analysis = vector_length_proofs(func, types);
    for op in func.blocks.iter().flat_map(|block| &block.ops) {
        let OpKind::Call {
            args,
            semantic: CallSemantic::Standard(StandardOperation::VecWithCapacity),
            ..
        } = &op.kind
        else {
            continue;
        };
        let (Some(value), Some(capacity_value)) = (op.result, args.first().copied()) else {
            continue;
        };
        let Some(vector) = func
            .values
            .get(value.0 as usize)
            .and_then(|value| types.get(value.ty))
        else {
            continue;
        };
        if vector.meta.candidate != RepresentationCandidate::Vec {
            continue;
        }
        let element = match &vector.kind {
            TypeKind::Adt { args, .. } => args.first().copied(),
            _ => None,
        };
        let element_name = element
            .map(|element| scalar_name(scalar_kind(types, element).unwrap_or(ScalarKind::Opaque)))
            .unwrap_or_else(|| "unknown".to_owned());
        let typed_array = element.and_then(|element| typed_array_name(types, element));
        let constant_initial_capacity = constant_u64(func, types, capacity_value);
        let length = analysis.proofs.get(&value).copied().unwrap_or_default();
        let use_decline = typed_storage_use_decline(func, types, abi, value, &analysis.aliases);
        let mut reason = storage_decline(
            types,
            vector,
            element,
            typed_array,
            constant_initial_capacity,
            length.maximum,
            use_decline,
        );
        if select_typed_storage
            && reason == "economics.complete-workload-evidence-missing"
            && func.values[value.0 as usize].carrier != Carrier::NativeVec
        {
            reason = "legality.native-carrier-unavailable";
        }
        let selected =
            select_typed_storage && reason == "economics.complete-workload-evidence-missing";
        if selected {
            let storage = TypedStorage {
                constructor: typed_array.expect("the selected storage has a typed constructor"),
                allocation_length: constant_initial_capacity
                    .expect("the selected storage has an initial capacity")
                    .max(
                        length
                            .maximum
                            .expect("the selected storage has a maximum length"),
                    ),
            };
            for (alias, root) in &analysis.aliases {
                if *root == value {
                    typed_values.insert(*alias, storage);
                }
            }
        }
        decisions.push(StorageDecision {
            item: name.clone(),
            value,
            element: element_name,
            typed_array,
            constant_initial_capacity,
            proven_maximum_length: length.maximum,
            proven_exact_final_length: length.current,
            selected,
            reason: (!selected).then_some(reason),
        });
    }
    (decisions, typed_values)
}

fn storage_decline(
    types: &TypeTable,
    vector: &crate::jir::types::TypeDescriptor,
    element: Option<TypeId>,
    typed_array: Option<&'static str>,
    constant_initial_capacity: Option<u64>,
    proven_maximum_length: Option<u64>,
    use_decline: Option<&'static str>,
) -> &'static str {
    match vector.meta.allocator {
        AllocatorIdentity::Global => {}
        AllocatorIdentity::Custom => return "legality.custom-allocator",
        AllocatorIdentity::Unknown | AllocatorIdentity::NotApplicable => {
            return "legality.allocator-unproven";
        }
    }
    match vector.meta.boundary {
        PinnedBoundary::None => {}
        PinnedBoundary::Host => return "boundary.host",
        PinnedBoundary::ViewAbi => return "boundary.view-abi",
    }
    let Some(element) = element.and_then(|element| types.get(element)) else {
        return "legality.element-type-unavailable";
    };
    if element.layout.needs_drop {
        return "legality.element-needs-drop";
    }
    if !element.layout.sized {
        return "legality.element-unsized";
    }
    if element.layout.zst {
        return "legality.element-zero-sized";
    }
    if typed_array.is_none() {
        return "legality.element-type-unsupported";
    }
    if let Some(reason) = use_decline {
        return reason;
    }
    if constant_initial_capacity.is_none() {
        return "analysis.constant-initial-capacity-unavailable";
    }
    if proven_maximum_length.is_none() {
        return "analysis.maximum-length-unavailable";
    }
    let Some(element_size) = element.layout.size.filter(|size| *size > 0) else {
        return "legality.element-layout-unavailable";
    };
    let maximum_capacity = (i32::MAX as u64) / element_size;
    if constant_initial_capacity.is_some_and(|capacity| capacity > maximum_capacity)
        || proven_maximum_length.is_some_and(|length| length > maximum_capacity)
    {
        return "legality.capacity-out-of-range";
    }
    // The synthetic request-byte kernel establishes a ceiling, not a promotion receipt. Keep
    // every otherwise legal reserved candidate observable until a complete compiled workload wins
    // in multiple engines, as required by the Phase 7 stop rule.
    "economics.complete-workload-evidence-missing"
}

fn typed_array_name(types: &TypeTable, ty: TypeId) -> Option<&'static str> {
    match types
        .get(types.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Integer {
            signed: false,
            bits: 8,
        }) => Some("Uint8Array"),
        Some(TypeKind::Integer {
            signed: true,
            bits: 8,
        }) => Some("Int8Array"),
        Some(TypeKind::Integer {
            signed: false,
            bits: 16,
        }) => Some("Uint16Array"),
        Some(TypeKind::Integer {
            signed: true,
            bits: 16,
        }) => Some("Int16Array"),
        Some(TypeKind::Integer {
            signed: false,
            bits: 32,
        }) => Some("Uint32Array"),
        Some(TypeKind::Integer {
            signed: true,
            bits: 32,
        }) => Some("Int32Array"),
        Some(TypeKind::Float { bits: 32 }) => Some("Float32Array"),
        Some(TypeKind::Float { bits: 64 }) => Some("Float64Array"),
        _ => None,
    }
}

/// The first use that prevents the narrow exact-buffer lowering.
///
/// The initial executable tranche keeps the vector private and supports only operations whose
/// existing native bodies work over indexed numeric storage. Capacity/layout observation, an
/// owned clone, a retained slice, removal/extension, raw access, and every unadapted call or
/// return stay explicit declines. This is intentionally stricter than `NativeVec` legality.
fn typed_storage_use_decline(
    func: &Func,
    types: &TypeTable,
    abi: &AbiPlan,
    root: ValueId,
    aliases: &BTreeMap<ValueId, ValueId>,
) -> Option<&'static str> {
    let is_root = |value: &ValueId| aliases.get(value) == Some(&root);
    for block in &func.blocks {
        for op in &block.ops {
            if !op.kind.uses().iter().any(&is_root) {
                continue;
            }
            match &op.kind {
                OpKind::Move(_) | OpKind::Deref(_) => {}
                OpKind::Copy(source) => {
                    let reference = func
                        .values
                        .get(source.0 as usize)
                        .and_then(|value| types.get(types.peeled(value.ty)))
                        .is_some_and(|descriptor| {
                            matches!(
                                descriptor.kind,
                                TypeKind::Reference { .. } | TypeKind::RawPointer { .. }
                            )
                        });
                    if !reference {
                        return Some("legality.identity-observed");
                    }
                }
                OpKind::BorrowView { path, .. } if path.is_empty() => {}
                OpKind::BorrowView { .. } | OpKind::RawView { .. } | OpKind::SliceView { .. } => {
                    return Some("legality.raw-parts-or-address-observed");
                }
                OpKind::Drop {
                    root: Root::Value(value),
                    path,
                    ..
                } if path.is_empty() && is_root(value) => {}
                OpKind::Call {
                    args,
                    semantic: CallSemantic::Standard(operation),
                    host: None,
                    ..
                } if args.first().is_some_and(&is_root)
                    && args.iter().skip(1).all(|value| !is_root(value)) =>
                {
                    match operation {
                        StandardOperation::VecPush
                        | StandardOperation::VecClear
                        | StandardOperation::VecLen
                        | StandardOperation::VecIsEmpty
                        | StandardOperation::VecIndex
                        | StandardOperation::VecIndexMut
                        | StandardOperation::VecDrop => {}
                        StandardOperation::VecCapacity => {
                            return Some("legality.capacity-or-layout-observed");
                        }
                        StandardOperation::VecAsSlice
                            if op.result.is_some_and(|slice| {
                                slice_has_only_scalar_consumers(func, abi, slice)
                            }) => {}
                        StandardOperation::VecAsSlice | StandardOperation::VecAsMutSlice => {
                            return Some("legality.retained-view");
                        }
                        StandardOperation::VecRemove | StandardOperation::VecExtend => {
                            return Some("lower.storage-operation-unadapted");
                        }
                        _ => return Some("boundary.standard-operation-unadapted"),
                    }
                }
                OpKind::Call { host: Some(_), .. } | OpKind::Host { .. } => {
                    return Some("boundary.host");
                }
                OpKind::Call { .. } | OpKind::Intrinsic { .. } => {
                    return Some("boundary.call-unadapted");
                }
                OpKind::Field { .. } | OpKind::SetField { .. } => {
                    return Some("legality.capacity-or-layout-observed");
                }
                OpKind::Drop { .. }
                | OpKind::Aggregate { .. }
                | OpKind::Repeat { .. }
                | OpKind::CellLoad(_)
                | OpKind::CellStore { .. }
                | OpKind::StoreThrough { .. }
                | OpKind::Materialize(_)
                | OpKind::Convert { .. } => return Some("boundary.storage-shape-unadapted"),
                _ => return Some("legality.identity-observed"),
            }
        }
        match &block.term {
            RTerm::Return(Some(value)) | RTerm::Throw(Some(value)) if is_root(value) => {
                return Some("boundary.return-unadapted");
            }
            RTerm::Switch { scrut, .. } if is_root(scrut) => {
                return Some("legality.identity-observed");
            }
            _ => {}
        }
    }
    None
}

/// Whether a semantic slice value disappears entirely into selected `buf, off, len` call lanes.
///
/// Moves, copies, and straight block-argument forwarding preserve the same SSA value for this
/// purpose. Every terminal use must be a slice position of a selected direct-call ABI; an
/// ordinary call, local inspection, return, store, or branch observation keeps the retained-view
/// decline. The callee-side ABI proof is responsible for the private lane boundary itself.
fn slice_has_only_scalar_consumers(func: &Func, abi: &AbiPlan, slice: ValueId) -> bool {
    let mut aliases = BTreeSet::from([slice]);
    loop {
        let before = aliases.len();
        for block in &func.blocks {
            for op in &block.ops {
                let source = match &op.kind {
                    OpKind::Move(source) | OpKind::Copy(source) => Some(*source),
                    _ => None,
                };
                if source.is_some_and(|source| aliases.contains(&source)) {
                    if let Some(result) = op.result {
                        aliases.insert(result);
                    }
                }
            }
            for (target, arguments) in block.term.edges() {
                let Some(target) = func.blocks.get(target.0 as usize) else {
                    return false;
                };
                for (parameter, argument) in target.params.iter().zip(arguments) {
                    if aliases.contains(argument) {
                        aliases.insert(*parameter);
                    }
                }
            }
        }
        if aliases.len() == before {
            break;
        }
    }

    let mut consumed = false;
    for block in &func.blocks {
        for op in &block.ops {
            match &op.kind {
                OpKind::Move(source) | OpKind::Copy(source) if aliases.contains(source) => {}
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    function: None,
                    args,
                    host: None,
                    ..
                } => {
                    let used: Vec<usize> = args
                        .iter()
                        .enumerate()
                        .filter_map(|(position, value)| aliases.contains(value).then_some(position))
                        .collect();
                    if used.is_empty() {
                        continue;
                    }
                    let Some(signature) = abi.call_signature(*item) else {
                        return false;
                    };
                    let positions = signature.slice_param_positions();
                    if !used.iter().all(|position| positions.contains(position)) {
                        return false;
                    }
                    consumed = true;
                }
                _ if op.kind.uses().iter().any(|value| aliases.contains(value)) => {
                    return false;
                }
                _ => {}
            }
        }
        if !matches!(&block.term, RTerm::Goto(..))
            && block
                .term
                .uses()
                .iter()
                .any(|value| aliases.contains(value))
        {
            return false;
        }
    }
    consumed
}

/// Proves logical vector lengths only for one straight-line control-flow path.
///
/// This is intentionally smaller than a general range analysis. Constructors begin empty;
/// moves and references preserve identity; the registered length-changing Vec operations update
/// the count. A branch, an opaque use, an invalid remove, or arithmetic overflow forgets the
/// affected proof. Copies of an owned Vec are independent allocations and therefore do not alias
/// their source.
fn vector_length_proofs(func: &Func, types: &TypeTable) -> VectorAnalysis {
    let mut aliases = BTreeMap::new();
    let mut proofs = BTreeMap::new();
    let mut visited = vec![false; func.blocks.len()];
    let mut block_index = 0_usize;

    loop {
        let Some(block) = func.blocks.get(block_index) else {
            forget_length_proofs(&mut proofs);
            break;
        };
        if visited[block_index] {
            forget_length_proofs(&mut proofs);
            break;
        }
        visited[block_index] = true;

        for op in &block.ops {
            let mut handled = false;
            if let OpKind::Call {
                args,
                semantic: CallSemantic::Standard(operation),
                ..
            } = &op.kind
            {
                match operation {
                    StandardOperation::VecNew | StandardOperation::VecWithCapacity => {
                        if let Some(result) = op.result {
                            aliases.insert(result, result);
                            proofs.insert(result, LengthProof::EMPTY);
                        }
                        handled = true;
                    }
                    StandardOperation::VecPush => {
                        if let Some(root) =
                            args.first().and_then(|value| aliases.get(value)).copied()
                        {
                            proofs.entry(root).or_default().push();
                        }
                        handled = true;
                    }
                    StandardOperation::VecClear => {
                        if let Some(root) =
                            args.first().and_then(|value| aliases.get(value)).copied()
                        {
                            proofs.entry(root).or_default().clear();
                        }
                        handled = true;
                    }
                    StandardOperation::VecRemove => {
                        if let Some(root) =
                            args.first().and_then(|value| aliases.get(value)).copied()
                        {
                            proofs.entry(root).or_default().remove();
                        }
                        handled = true;
                    }
                    StandardOperation::VecExtend => {
                        let target = args.first().and_then(|value| aliases.get(value)).copied();
                        let source = args.get(1).and_then(|value| aliases.get(value)).copied();
                        match (target, source) {
                            (Some(target), Some(source)) if target != source => {
                                let source_length =
                                    proofs.get(&source).and_then(|proof| proof.current);
                                let next = proofs
                                    .get(&target)
                                    .and_then(|proof| proof.current)
                                    .zip(source_length)
                                    .and_then(|(target, source)| target.checked_add(source));
                                match next {
                                    Some(next) => {
                                        if let Some(proof) = proofs.get_mut(&target) {
                                            proof.current = Some(next);
                                            proof.maximum =
                                                proof.maximum.map(|maximum| maximum.max(next));
                                        }
                                        if let Some(proof) = proofs.get_mut(&source) {
                                            proof.current = Some(0);
                                        }
                                    }
                                    None => {
                                        if let Some(proof) = proofs.get_mut(&target) {
                                            proof.forget();
                                        }
                                        if let Some(proof) = proofs.get_mut(&source) {
                                            proof.forget();
                                        }
                                    }
                                }
                            }
                            (Some(root), _) | (_, Some(root)) => {
                                proofs.entry(root).or_default().forget();
                            }
                            (None, None) => {}
                        }
                        handled = true;
                    }
                    StandardOperation::VecLen
                    | StandardOperation::VecCapacity
                    | StandardOperation::VecIsEmpty
                    | StandardOperation::VecAsSlice
                    | StandardOperation::VecAsMutSlice
                    | StandardOperation::VecIndex
                    | StandardOperation::VecIndexMut
                    | StandardOperation::VecDrop => handled = true,
                    _ => {}
                }
            }

            if !handled {
                let roots: Vec<ValueId> = op
                    .kind
                    .uses()
                    .into_iter()
                    .filter_map(|value| aliases.get(&value).copied())
                    .collect();
                let benign = matches!(
                    &op.kind,
                    OpKind::Move(_)
                        | OpKind::Copy(_)
                        | OpKind::BorrowView { .. }
                        | OpKind::Deref(_)
                        | OpKind::Field { .. }
                        | OpKind::Drop { .. }
                );
                if !benign {
                    for root in roots {
                        proofs.entry(root).or_default().forget();
                    }
                }
            }

            let Some(result) = op.result else { continue };
            let source = match &op.kind {
                OpKind::Move(source) | OpKind::Deref(source) => aliases.get(source).copied(),
                OpKind::Copy(source)
                    if func
                        .values
                        .get(source.0 as usize)
                        .and_then(|value| types.get(types.peeled(value.ty)))
                        .is_some_and(|descriptor| {
                            matches!(
                                descriptor.kind,
                                TypeKind::Reference { .. } | TypeKind::RawPointer { .. }
                            )
                        }) =>
                {
                    aliases.get(source).copied()
                }
                OpKind::BorrowView {
                    root: Root::Value(source),
                    path,
                    ..
                } if path.is_empty() => aliases.get(source).copied(),
                _ => None,
            };
            if let Some(root) = source {
                aliases.insert(result, root);
            }
        }

        match &block.term {
            RTerm::Goto(target, arguments) => {
                let Some(next) = func.blocks.get(target.0 as usize) else {
                    forget_length_proofs(&mut proofs);
                    break;
                };
                for (parameter, argument) in next.params.iter().zip(arguments) {
                    if let Some(root) = aliases.get(argument).copied() {
                        aliases.insert(*parameter, root);
                    }
                }
                block_index = target.0 as usize;
            }
            RTerm::Return(_) | RTerm::Unreachable | RTerm::Throw(_) => break,
            RTerm::Switch { .. } | RTerm::Invoke { .. } => {
                forget_length_proofs(&mut proofs);
                break;
            }
        }
    }

    VectorAnalysis { proofs, aliases }
}

fn forget_length_proofs(proofs: &mut BTreeMap<ValueId, LengthProof>) {
    for proof in proofs.values_mut() {
        proof.forget();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntegerConstant {
    raw: u128,
    signed: bool,
    bits: u32,
}

impl IntegerConstant {
    fn new(raw: u128, signed: bool, bits: u32) -> Option<Self> {
        (bits > 0 && bits <= 128).then(|| Self {
            raw: truncate(raw, bits),
            signed,
            bits,
        })
    }

    fn cast(self, signed: bool, bits: u32) -> Option<Self> {
        let extended = if self.signed && sign_bit(self.raw, self.bits) {
            self.raw | !mask(self.bits)
        } else {
            self.raw
        };
        Self::new(extended, signed, bits)
    }

    fn as_u64(self) -> Option<u64> {
        if self.signed && sign_bit(self.raw, self.bits) {
            return None;
        }
        u64::try_from(self.raw).ok()
    }
}

fn mask(bits: u32) -> u128 {
    if bits == 128 {
        u128::MAX
    } else {
        (1_u128 << bits) - 1
    }
}

fn truncate(value: u128, bits: u32) -> u128 {
    value & mask(bits)
}

fn sign_bit(value: u128, bits: u32) -> bool {
    value & (1_u128 << (bits - 1)) != 0
}

fn constant_u64(func: &Func, types: &TypeTable, mut value: ValueId) -> Option<u64> {
    let mut pending_casts = Vec::new();
    for _ in 0..=func.values.len() {
        let op = func
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .find(|op| op.result == Some(value))?;
        match &op.kind {
            OpKind::Const(ConstTree::Scalar { bits, .. }) => {
                let ty = func.values.get(value.0 as usize)?.ty;
                let (signed, width) = types.int_info(ty)?;
                let mut constant = IntegerConstant::new(*bits, signed, width)?;
                for (signed, width) in pending_casts.into_iter().rev() {
                    constant = constant.cast(signed, width)?;
                }
                return constant.as_u64();
            }
            OpKind::Copy(source) | OpKind::Move(source) => value = *source,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v,
                to,
                ..
            } => {
                pending_casts.push(types.int_info(*to)?);
                value = *v;
            }
            _ => return None,
        }
    }
    None
}

fn metrics_option_json(out: &mut String, metrics: Option<ProgramMetrics>) {
    let Some(metrics) = metrics else {
        out.push_str("null");
        return;
    };
    let score = metrics.min.saturating_add(metrics.alloc_term);
    let _ = write!(
        out,
        "{{\"min\":{},\"gzip\":{},\"brotli\":{},\"allocations\":{},\
         \"loop_allocations\":{},\"alloc_term\":{},\"score\":{}}}",
        metrics.min,
        metrics.gzip,
        metrics.brotli,
        metrics.allocations,
        metrics.loop_allocations,
        metrics.alloc_term,
        score,
    );
}

fn type_table<'a>(table: &'a BTreeMap<JsName, LinkItem>, name: &JsName) -> Option<&'a TypeTable> {
    table
        .get(name)
        .and_then(|item| item.jir.interns())
        .map(|interns| &interns.types)
}

fn param_shape(types: &TypeTable, param: &FlatParam) -> ValueShape {
    if param.slice {
        return ValueShape::Product(vec![
            LaneKind::Buffer,
            lane_kind(types, param.field_tys[1]),
            lane_kind(types, param.field_tys[2]),
        ]);
    }
    if let Some(tagged) = &param.tagged {
        return ValueShape::Product(
            std::iter::once(LaneKind::Tag)
                .chain(
                    tagged
                        .lanes
                        .iter()
                        .flat_map(|lane| product_lanes(types, &lane.node)),
                )
                .collect(),
        );
    }
    if let Some(recursive) = &param.recursive {
        return ValueShape::Product(product_lanes(types, &recursive.root));
    }
    ValueShape::Product(
        param
            .passed
            .iter()
            .map(|field| lane_kind(types, param.field_tys[*field as usize]))
            .collect(),
    )
}

fn result_shape(types: &TypeTable, result: &FlatResult) -> ValueShape {
    if let Some(tagged) = &result.tagged {
        return ValueShape::Product(
            std::iter::once(LaneKind::Tag)
                .chain(
                    tagged
                        .lanes
                        .iter()
                        .flat_map(|lane| product_lanes(types, &lane.node)),
                )
                .collect(),
        );
    }
    if let Some(recursive) = &result.recursive {
        return ValueShape::Product(product_lanes(types, &recursive.root));
    }
    ValueShape::Product(
        result
            .passed
            .iter()
            .map(|field| lane_kind(types, result.field_tys[*field as usize]))
            .collect(),
    )
}

fn product_lanes(types: &TypeTable, node: &ProductNode) -> Vec<LaneKind> {
    let mut components = Vec::new();
    node.lane_components(&mut components);
    components
        .into_iter()
        .map(|(ty, component)| match component {
            LaneComponent::Value => lane_kind(types, ty),
            LaneComponent::Buffer => LaneKind::Buffer,
            LaneComponent::Offset | LaneComponent::Length => {
                LaneKind::Scalar(ScalarKind::Unsigned(32))
            }
        })
        .collect()
}

fn value_shape(types: &TypeTable, ty: TypeId, carrier: Carrier) -> ValueShape {
    match carrier {
        Carrier::Baseline => scalar_kind(types, ty)
            .map(ValueShape::Scalar)
            .unwrap_or_else(|| {
                if types.is_zst(ty) {
                    ValueShape::Product(Vec::new())
                } else {
                    ValueShape::Materialized(Carrier::Baseline)
                }
            }),
        Carrier::HostString => ValueShape::Scalar(ScalarKind::HostString),
        Carrier::Virtual | Carrier::Flat => ValueShape::Product(Vec::new()),
        carrier => ValueShape::Materialized(carrier),
    }
}

fn lane_kind(types: &TypeTable, ty: TypeId) -> LaneKind {
    scalar_kind(types, ty)
        .map(LaneKind::Scalar)
        .unwrap_or(LaneKind::Value)
}

fn scalar_kind(types: &TypeTable, ty: TypeId) -> Option<ScalarKind> {
    match types
        .get(types.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Bool) => Some(ScalarKind::Bool),
        Some(TypeKind::Char) => Some(ScalarKind::Char),
        Some(TypeKind::Integer { signed: true, bits }) => Some(ScalarKind::Signed(*bits)),
        Some(TypeKind::Integer {
            signed: false,
            bits,
        }) => Some(ScalarKind::Unsigned(*bits)),
        Some(TypeKind::Float { bits }) => Some(ScalarKind::Float(*bits)),
        Some(
            TypeKind::Reference { .. }
            | TypeKind::RawPointer { .. }
            | TypeKind::FnDef { .. }
            | TypeKind::FnPtr { .. },
        ) => Some(ScalarKind::Reference),
        Some(TypeKind::Opaque) => Some(ScalarKind::Opaque),
        _ => None,
    }
}

fn storage_plan(types: &TypeTable, ty: TypeId, carrier: Carrier) -> StoragePlan {
    match carrier {
        Carrier::Virtual => StoragePlan::Virtual,
        Carrier::Flat | Carrier::HostString | Carrier::Opaque => StoragePlan::ScalarLocals,
        Carrier::NativeVec => StoragePlan::JsArray,
        Carrier::StringBuilder | Carrier::SliceView | Carrier::NativeBox => StoragePlan::JsObject,
        Carrier::Baseline if scalar_kind(types, ty).is_some() || types.is_zst(ty) => {
            StoragePlan::ScalarLocals
        }
        Carrier::Baseline => StoragePlan::Baseline,
    }
}

fn drop_plan(types: &TypeTable, ty: TypeId, carrier: Carrier) -> DropPlan {
    if !types
        .get(ty)
        .is_some_and(|descriptor| descriptor.layout.needs_drop)
    {
        DropPlan::Elided
    } else if matches!(carrier, Carrier::NativeVec | Carrier::NativeBox) {
        DropPlan::Direct
    } else {
        DropPlan::Baseline
    }
}

fn drop_reason(plan: DropPlan, carrier: Carrier) -> &'static str {
    match (plan, carrier) {
        (DropPlan::Direct, Carrier::NativeVec) => "registry.native-vec-drop",
        (DropPlan::Direct, Carrier::NativeBox) => "registry.native-box-drop",
        (DropPlan::Baseline, _) => "cleanup.control-flow-proof-unavailable",
        (DropPlan::Elided, _) => "cleanup.not-required",
        _ => "cleanup.capability-not-selected",
    }
}

fn shape_json(out: &mut String, shape: &ValueShape) {
    match shape {
        ValueShape::Scalar(kind) => {
            out.push_str("{\"scalar\":");
            push_string(out, scalar_name(*kind).as_str());
            out.push('}');
        }
        ValueShape::Product(lanes) => {
            out.push_str("{\"product\":[");
            for (index, lane) in lanes.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                match lane {
                    LaneKind::Scalar(kind) => push_string(out, scalar_name(*kind).as_str()),
                    LaneKind::Tag => push_string(out, "tag"),
                    LaneKind::Buffer => push_string(out, "buffer"),
                    LaneKind::Value => push_string(out, "value"),
                }
            }
            out.push_str("]}");
        }
        ValueShape::Materialized(carrier) => {
            out.push_str("{\"materialized\":");
            push_string(out, carrier_name(*carrier));
            out.push('}');
        }
    }
}

fn counts_json<K: Copy + Ord>(
    out: &mut String,
    counts: &BTreeMap<K, u64>,
    name: fn(K) -> &'static str,
) {
    for (index, (plan, count)) in counts.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"plan\":");
        push_string(out, name(*plan));
        let _ = write!(out, ",\"values\":{count}}}");
    }
}

fn scalar_name(kind: ScalarKind) -> String {
    match kind {
        ScalarKind::Bool => "bool".to_owned(),
        ScalarKind::Char => "char".to_owned(),
        ScalarKind::Signed(bits) => format!("i{bits}"),
        ScalarKind::Unsigned(bits) => format!("u{bits}"),
        ScalarKind::Float(bits) => format!("f{bits}"),
        ScalarKind::Reference => "reference".to_owned(),
        ScalarKind::HostString => "host-string".to_owned(),
        ScalarKind::Opaque => "opaque".to_owned(),
    }
}

fn carrier_name(carrier: Carrier) -> &'static str {
    match carrier {
        Carrier::Baseline => "baseline",
        Carrier::Virtual => "virtual",
        Carrier::Flat => "flat",
        Carrier::HostString => "host-string",
        Carrier::StringBuilder => "string-builder",
        Carrier::NativeVec => "native-vec",
        Carrier::SliceView => "slice-view",
        Carrier::Opaque => "opaque",
        Carrier::NativeBox => "native-box",
    }
}

fn wrapper_name(policy: WrapperPolicy) -> &'static str {
    match policy {
        WrapperPolicy::None => "none",
        WrapperPolicy::Canonical => "canonical",
    }
}

fn storage_name(plan: StoragePlan) -> &'static str {
    match plan {
        StoragePlan::Virtual => "virtual",
        StoragePlan::ScalarLocals => "scalar-locals",
        StoragePlan::AddressableCells => "addressable-cells",
        StoragePlan::JsArray => "js-array",
        StoragePlan::TypedArray => "typed-array",
        StoragePlan::JsObject => "js-object",
        StoragePlan::Baseline => "baseline",
    }
}

fn drop_name(plan: DropPlan) -> &'static str {
    match plan {
        DropPlan::Elided => "elided",
        DropPlan::Direct => "direct",
        DropPlan::TryFinally => "try-finally",
        DropPlan::JsDispose => "js-dispose",
        DropPlan::JsAsyncDispose => "js-async-dispose",
        DropPlan::DisposableStack => "disposable-stack",
        DropPlan::Baseline => "baseline",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_names_all_four_physical_decisions() {
        let plan = PhysicalPlan {
            ready_futures: BTreeSet::new(),
            calls: 1,
            selected_calls: vec![CallAbi {
                item: JsName::new("pair$h1".to_owned()),
                params: vec![ValueShape::Product(vec![
                    LaneKind::Scalar(ScalarKind::Unsigned(32)),
                    LaneKind::Scalar(ScalarKind::Unsigned(32)),
                ])],
                result: ValueShape::Scalar(ScalarKind::Unsigned(32)),
                wrapper: WrapperPolicy::None,
            }],
            decisions: vec![
                AbiDecision {
                    item: JsName::new("pair$h1".to_owned()),
                    shape: Some("product(2)".to_owned()),
                    selected: true,
                    reason: None,
                    cross_crate: Default::default(),
                },
                AbiDecision {
                    item: JsName::new("stays$h2".to_owned()),
                    shape: None,
                    selected: false,
                    reason: Some("boundary.result-use".to_owned()),
                    cross_crate: Default::default(),
                },
            ],
            storage: [(StoragePlan::ScalarLocals, 2)].into_iter().collect(),
            drop: [(DropPlan::Elided, 2)].into_iter().collect(),
            typed_storage_enabled: false,
            storage_decisions: Vec::new(),
            typed_values: BTreeMap::new(),
            drop_decisions: Vec::new(),
            economics: None,
        };
        assert_eq!(
            plan.json(),
            "{\"mode\":\"exact-compiler-link-artifact\",\"calls\":1,\"profile\":null,\"canonical\":null,\"planned\":null,\"validated\":null,\"final_emitted\":null,\"cache_hits\":0,\"selected_calls\":[{\"item\":\"pair$h1\",\"params\":[{\"product\":[\"u32\",\"u32\"]}],\"result\":{\"scalar\":\"u32\"},\"wrapper\":\"none\"}],\"decisions\":[{\"item\":\"pair$h1\",\"shape\":\"product(2)\",\"profile\":null,\"selected\":true,\"reason\":null,\"before\":null,\"after\":null,\"hotness_source\":null,\"budget_consumed_bps\":0,\"cross_crate\":{\"summary_available\":false,\"summary_validated\":false,\"body_decoded\":false,\"caller_count\":0,\"crosses_crate\":false,\"producer_crate\":null,\"caller_crates\":[],\"selected\":true,\"reason\":null}},{\"item\":\"stays$h2\",\"shape\":null,\"profile\":null,\"selected\":false,\"reason\":\"boundary.result-use\",\"before\":null,\"after\":null,\"hotness_source\":null,\"budget_consumed_bps\":0,\"cross_crate\":{\"summary_available\":false,\"summary_validated\":false,\"body_decoded\":false,\"caller_count\":0,\"crosses_crate\":false,\"producer_crate\":null,\"caller_crates\":[],\"selected\":false,\"reason\":\"boundary.result-use\"}}],\"storage\":[{\"plan\":\"scalar-locals\",\"values\":2}],\"drop\":[{\"plan\":\"elided\",\"values\":2}],\"storage_mode\":\"report-only\",\"storage_decisions\":[],\"cleanup_mode\":\"existing-executable\",\"drop_decisions\":[]}"
        );
    }

    #[test]
    fn cast_derived_capacity_respects_width_and_signedness() {
        use crate::jir::origin::OriginId;
        use crate::jir::repr::{FuncBuilder, RTerm};
        use crate::jir::semantic::{AbiSignature, CastKind, DefinitionId, ItemId, Signature};
        use crate::jir::types::{
            IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeMeta,
        };

        fn integer(id: TypeId, signed: bool, bits: u32) -> TypeDescriptor {
            TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Integer { signed, bits },
                layout: LayoutFacts {
                    size: Some(u64::from(bits / 8)),
                    align: Some(u64::from(bits / 8)),
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: Some(IntFacts {
                        signed,
                        bits,
                        repr: if bits <= 32 {
                            IntRepr::Number
                        } else {
                            IntRepr::BigInt
                        },
                    }),
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: id,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: bits == 8,
                    fields: Vec::new(),
                },
            }
        }

        let i8_ty = TypeId(1);
        let u8_ty = TypeId(2);
        let i32_ty = TypeId(3);
        let u32_ty = TypeId(4);
        let mut types = TypeTable::new();
        for descriptor in [
            integer(i8_ty, true, 8),
            integer(u8_ty, false, 8),
            integer(i32_ty, true, 32),
            integer(u32_ty, false, 32),
        ] {
            types.insert(descriptor);
        }
        let mut builder = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 1,
                    local_hash: 1,
                },
                instantiation: 1,
            },
            Signature {
                arguments: Vec::new(),
                result: u32_ty,
                abi: AbiSignature::default(),
            },
        );
        let unsigned = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar {
                bits: 0xff,
                bytes: 1,
            }),
            u8_ty,
            OriginId(0),
        );
        let widened_unsigned = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: unsigned,
                from: u8_ty,
                to: u32_ty,
            },
            u32_ty,
            OriginId(0),
        );
        let negative = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Scalar {
                bits: 0xff,
                bytes: 1,
            }),
            i8_ty,
            OriginId(0),
        );
        let widened_negative = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: negative,
                from: i8_ty,
                to: u32_ty,
            },
            u32_ty,
            OriginId(0),
        );
        let narrowed_negative = builder.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::IntToInt,
                v: widened_negative,
                from: u32_ty,
                to: i8_ty,
            },
            i8_ty,
            OriginId(0),
        );
        builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(widened_unsigned)));
        let func = builder.finish();

        assert_eq!(constant_u64(&func, &types, unsigned), Some(255));
        assert_eq!(constant_u64(&func, &types, widened_unsigned), Some(255));
        assert_eq!(
            constant_u64(&func, &types, widened_negative),
            Some(u64::from(u32::MAX))
        );
        assert_eq!(constant_u64(&func, &types, narrowed_negative), None);
    }

    #[test]
    fn straight_line_vec_lengths_cover_push_remove_extend_and_clear() {
        use crate::jir::origin::OriginId;
        use crate::jir::repr::{FuncBuilder, RTerm};
        use crate::jir::semantic::{AbiSignature, Callee, DefinitionId, ItemId, Signature};
        use crate::jir::StrId;

        let vector_ty = TypeId(1);
        let vector_ref_ty = TypeId(2);
        let usize_ty = TypeId(3);
        let element_ty = TypeId(4);
        let unit_ty = TypeId(5);
        let item = ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 1,
            },
            instantiation: 1,
        };
        let callee = || Callee::Direct {
            item,
            link_name: StrId(0),
        };
        let call = |operation, args| OpKind::Call {
            callee: callee(),
            function: None,
            args,
            track_caller: None,
            semantic: CallSemantic::Standard(operation),
            host: None,
        };
        let mut builder = FuncBuilder::new(
            item,
            Signature {
                arguments: Vec::new(),
                result: unit_ty,
                abi: AbiSignature::default(),
            },
        );
        let block = FuncBuilder::ENTRY;
        let first = builder.op(
            block,
            call(StandardOperation::VecNew, Vec::new()),
            vector_ty,
            OriginId(0),
        );
        let first_ref = builder.op(
            block,
            OpKind::BorrowView {
                root: Root::Value(first),
                path: Vec::new(),
                mutable: true,
            },
            vector_ref_ty,
            OriginId(0),
        );
        let element = builder.op(
            block,
            OpKind::Const(ConstTree::Scalar { bits: 7, bytes: 4 }),
            element_ty,
            OriginId(0),
        );
        for _ in 0..3 {
            builder.effect(
                block,
                call(StandardOperation::VecPush, vec![first_ref, element]),
                OriginId(0),
            );
        }
        let index = builder.op(
            block,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            usize_ty,
            OriginId(0),
        );
        builder.op(
            block,
            call(StandardOperation::VecRemove, vec![first_ref, index]),
            element_ty,
            OriginId(0),
        );

        let capacity = builder.op(
            block,
            OpKind::Const(ConstTree::Scalar { bits: 8, bytes: 4 }),
            usize_ty,
            OriginId(0),
        );
        let second = builder.op(
            block,
            call(StandardOperation::VecWithCapacity, vec![capacity]),
            vector_ty,
            OriginId(0),
        );
        let second_ref = builder.op(
            block,
            OpKind::BorrowView {
                root: Root::Value(second),
                path: Vec::new(),
                mutable: true,
            },
            vector_ref_ty,
            OriginId(0),
        );
        builder.effect(
            block,
            call(StandardOperation::VecPush, vec![second_ref, element]),
            OriginId(0),
        );
        let moved_first = builder.op(block, OpKind::Move(first), vector_ty, OriginId(0));
        builder.effect(
            block,
            call(StandardOperation::VecExtend, vec![second_ref, moved_first]),
            OriginId(0),
        );
        builder.effect(
            block,
            call(StandardOperation::VecClear, vec![second_ref]),
            OriginId(0),
        );
        builder.terminate(block, RTerm::Return(None));

        let analysis = vector_length_proofs(&builder.finish(), &TypeTable::new());
        assert_eq!(
            analysis.proofs.get(&first),
            Some(&LengthProof {
                current: Some(0),
                maximum: Some(3),
            })
        );
        assert_eq!(
            analysis.proofs.get(&second),
            Some(&LengthProof {
                current: Some(0),
                maximum: Some(3),
            })
        );
    }
}
