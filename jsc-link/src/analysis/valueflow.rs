//! Whole-program value discovery and flow over JIR-R: the graph the representation planner
//! labels.
//!
//! This ports the discovery ideas of v1's `representation_plan.rs` (`discover_values`, the
//! `connect_*` family, `ValuePath`) onto [`Func`]s, with the one structural change the v2 plan is
//! built on (D2): **inside one item, names for the same value union into one node; across items,
//! every boundary is an edge.** v1 unioned across call boundaries too and then poisoned whole
//! components; here a boundary edge carries its own constraint and nothing propagates by
//! contagion.
//!
//! # Nodes
//!
//! A node is one representation-candidate value at one path: `(item, SSA value or cell or the
//! return slot, path)`, where a path step is a field, an enum variant's field, a deref, or a vec
//! or slice element. Discovery walks the value's type exactly as v1 walked type definitions:
//! tuples, single-variant structs, enum variants, closure captures, references, the element of a
//! candidate `Vec`, and slice elements; a `String` or `Box` is a boundary and its internals are
//! not walked. Beside the value nodes there are boundary nodes: the unseen slots of a callee with
//! no lowered body ([`NodeKey::Opaque`]), the shared slots of an indirect-dispatch group
//! ([`NodeKey::Group`]), a static's storage ([`NodeKey::Static`]), and the single
//! [`NodeKey::Baseline`] anchor a pinned edge ends at.
//!
//! # Unions and edges
//!
//! Copies, moves, block arguments, cell loads and stores, field projections and aggregations,
//! borrows, and derefs union intra-item names by matching path suffixes (v1's prefix flows). A
//! call, a drop, a host slot, a static reference, an indirect dispatch, and an export round trip
//! are edges. An edge whose path crosses a `Deref` shares storage and is [`EdgeKind::Shared`]
//! (must-equal); a by-value crossing is [`EdgeKind::Value`], the adapter edge, carrying whether
//! the source was proven dead and unaliased after the edge (the baseline-to-native direction's
//! legality). Host slots are classified per slot by
//! [`crate::analysis::host_contract::HostContract`]: the pin lands on the one edge, never on the
//! item.
//!
//! # Soundness defaults
//!
//! What discovery cannot model it pins locally, never regionally: an unmodelable projection or an
//! aggregation into an unwalked type marks the values it can see
//! ([`InfReason::UnsupportedOperation`]); a `dyn` coercion, a dynamic drop, and the receiver of a
//! vtable method mark [`InfReason::UnknownCall`]. A fn-pointer dispatch group additionally pins
//! its own slots, because reifications inside unlowered items cannot be enumerated; a vtable
//! group's targets are complete (every reachable vtable body is in the table), so it is not
//! pinned. Bodies with a standard-operation semantic are summarized (v1
//! `summarize_standard_operation`) instead of collected, so their raw internals never poison
//! their callers' edges.
//!
//! # The region cut (Phase 5, D2)
//!
//! A registry-covered `CallSemantic::Standard` call is a region boundary: its deref-crossing
//! paths make **no** edge into the callee at all — whatever the callee's own `BodySemantic` —
//! and the registry's slot requirements are applied to the caller's own values instead
//! (mutability from the requirement's reference kind, a carrier-set constraint on the slot's
//! node, and the operation's slot flows run caller-side). By-value paths keep their priced
//! adapter edges. The plan adds asymmetric coherence arcs so the callee's one body can never go
//! native beside a baseline caller, and the commit refuses a label disagreement at a reference
//! slot; an untagged alloc-internal body is thereby never must-equal-joined to app values and
//! declines only its own region.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::analysis::host_contract::{HostContract, SlotTerm};
use crate::analysis::legality::InfReason;
use crate::analysis::refs::{self, RefScan};
use crate::descriptor::{Descriptor, Slot, SlotFacts};
use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::repr::{Carrier, CellId, Func, OpKind, Proj, RTerm, Root, ValueId};
use crate::jir::semantic::{
    BodyKind, BodySemantic, CallSemantic, Callee, CastKind, ConstTree, DefinitionId, DropGlue,
    GlobalReference, HostParameterClass, HostPayload, ItemId, StandardOperation, VtableEntry,
};
use crate::jir::types::{NominalForm, TypeDescriptor, TypeKind, TypeTable};
use crate::jir::{Interns, TypeId};
use crate::late_ir::AllocatorProvider;
use crate::lower::semantic_ops::string::{
    analyze_inlined_string_vec, bstr_spec, classify_bstr_op, is_raw_vec_reserve_in_table,
    item_origin_info, StringRoot,
};
use crate::lower::semantic_ops::{
    self, ReferenceRequirement, SemanticOp, SignatureSlot, SlotRequirement,
};

/// One step of a value path.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Step {
    Deref,
    Field(u32),
    /// The following `Field` steps read the enum variant with this index.
    Variant(u32),
    /// The element of a candidate `Vec` or of a slice.
    Element,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deref => write!(f, "deref"),
            Self::Field(index) => write!(f, "f{index}"),
            Self::Variant(index) => write!(f, "v{index}"),
            Self::Element => write!(f, "elem"),
        }
    }
}

/// The candidate family of a discovered value, from [`crate::jir::types::TypeMeta::candidate`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CandidateKind {
    Box,
    Vec,
    String,
}

/// Where a value node is anchored inside one item.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Site {
    Value(ValueId),
    Cell(CellId),
    /// The item's return slot: every returned value unions into it, and callers edge to it.
    Ret,
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(v) => write!(f, "v{}", v.0),
            Self::Cell(c) => write!(f, "c{}", c.0),
            Self::Ret => write!(f, "ret"),
        }
    }
}

/// A parameter or return slot of a boundary the analysis cannot see into.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BoundarySlot {
    Param(u32),
    Ret,
}

impl fmt::Display for BoundarySlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Param(index) => write!(f, "p{index}"),
            Self::Ret => write!(f, "ret"),
        }
    }
}

/// One indirect-dispatch group: all targets of one fn-pointer type, or of one vtable slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GroupKey {
    /// Dispatch through a function pointer of this type.
    FnPtr(TypeId),
    /// Dispatch through vtable slot `slot` of the trait `trait_ref` (`None` for the header-only
    /// vtable of a `dyn Send`-shaped object, which has no method slots to dispatch).
    Vtable {
        trait_ref: Option<DefinitionId>,
        slot: u32,
    },
}

/// One node of the value graph.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NodeKey {
    /// A discovered candidate value inside a lowered item.
    Value {
        item: JsName,
        site: Site,
        path: Vec<Step>,
    },
    /// A slot of a callee whose body the lane did not lower: a declined item, or a foreign
    /// symbol. Carries a unary reason on its facts.
    Opaque {
        item: String,
        slot: BoundarySlot,
        path: Vec<Step>,
    },
    /// A slot of an indirect-dispatch group.
    Group {
        key: GroupKey,
        slot: BoundarySlot,
        path: Vec<Step>,
    },
    /// A static's storage; always [`InfReason::StaticEscape`].
    Static {
        definition: DefinitionId,
        path: Vec<Step>,
    },
    /// The anchor a pinned edge ends at: always labeled baseline.
    Baseline,
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = |f: &mut fmt::Formatter<'_>, path: &[Step]| {
            for step in path {
                write!(f, ".{step}")?;
            }
            Ok(())
        };
        match self {
            Self::Value {
                item,
                site,
                path: p,
            } => {
                write!(f, "{}/{site}", item.as_str())?;
                path(f, p)
            }
            Self::Opaque {
                item,
                slot,
                path: p,
            } => {
                write!(f, "opaque:{item}/{slot}")?;
                path(f, p)
            }
            Self::Group { key, slot, path: p } => {
                match key {
                    GroupKey::FnPtr(ty) => write!(f, "group:fnptr:{:032x}/{slot}", ty.0)?,
                    GroupKey::Vtable { trait_ref, slot: v } => {
                        match trait_ref {
                            Some(def) => write!(
                                f,
                                "group:vtable:{:016x}.{:016x}:{v}/{slot}",
                                def.crate_id, def.local_hash
                            )?,
                            None => write!(f, "group:vtable:auto:{v}/{slot}")?,
                        };
                    }
                }
                path(f, p)
            }
            Self::Static {
                definition,
                path: p,
            } => {
                write!(
                    f,
                    "static:{:016x}.{:016x}",
                    definition.crate_id, definition.local_hash
                )?;
                path(f, p)
            }
            Self::Baseline => write!(f, "baseline"),
        }
    }
}

/// The definition-site facts of one node.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NodeFacts {
    /// The candidate family, `None` for the baseline anchor.
    pub kind: Option<CandidateKind>,
    /// The Rust type at the node's path, where one is known.
    pub ty: Option<TypeId>,
    /// Whether anything writes the value: what separates a builder from an immutable carrier.
    pub mutable: bool,
    /// The unary legality reasons; any entry makes the node's native label infinite.
    pub reasons: BTreeSet<InfReason>,
    /// The carriers a native label may take, intersected over every registry-covered standard
    /// call whose slot requirement lands on the node (D2's cut). `None` leaves the family
    /// sub-solves free; legality turns a singleton into a [`crate::analysis::legality::Term::
    /// FixedCarrier`] and an empty set into a pin.
    pub carriers: Option<BTreeSet<Carrier>>,
}

/// How an edge constrains its two endpoints.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EdgeKind {
    /// A by-value boundary crossing from `a` to `b`: the adapter edge. `source_dead` is whether
    /// the source value was proven dead and unaliased after the edge, which is what makes the
    /// baseline-to-native direction legal.
    Value { source_dead: bool },
    /// The endpoints share storage through a pointer: must-equal.
    Shared,
    /// A host slot whose contract pins: must-equal against [`NodeKey::Baseline`].
    HostPin,
    /// A host slot whose contract only reads a shape it declared it accepts. `accepts` is the
    /// declaration's carrier set (`descriptor::SlotFacts`): the edge is free exactly for the
    /// carriers in it, and legality pins or fixes the slot's value where a solvable carrier
    /// falls outside it.
    HostSink { accepts: SlotFacts },
    /// An export round trip: must-equal.
    Export,
}

/// One boundary edge, `a` the source side.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Edge {
    pub a: NodeKey,
    pub b: NodeKey,
    pub kind: EdgeKind,
}

/// The whole-program value graph.
#[derive(Clone, Debug, Default)]
pub struct ValueGraph {
    pub nodes: BTreeMap<NodeKey, NodeFacts>,
    /// Intra-item same-value pairs, `(min, max)` ordered.
    pub unions: BTreeSet<(NodeKey, NodeKey)>,
    pub edges: BTreeSet<Edge>,
    /// Caller to callee over plain direct calls and direct drop glue: the SCC input for the
    /// version facility nodes.
    pub direct_calls: BTreeMap<JsName, BTreeSet<JsName>>,
    /// The nodes [`InfReason::UnknownCall`]-pinned by the item-boundary walk ([`Collector::pins`])
    /// alone: the pins one `$n` clone would bypass, which is what the sidecar's
    /// `pinned_cost_positive` accounting (D10b) counts regions blocked by.
    pub pinned_boundaries: BTreeSet<NodeKey>,
    /// The candidate slot nodes of exported boundaries: the sidecar's `repr.exports` block
    /// publishes each one's committed carrier — the export-representation contract an outside
    /// caller owes (v1 published the same through its report).
    pub exported_slots: BTreeSet<NodeKey>,
}

impl ValueGraph {
    /// A deterministic text dump: the fixture and review format.
    #[allow(dead_code)] // The unit tests and reviews read it; builds do not.
    pub fn dump(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for (key, facts) in &self.nodes {
            let _ = write!(out, "node {key}");
            if let Some(kind) = facts.kind {
                let _ = write!(out, " {kind:?}");
            }
            if facts.mutable {
                out.push_str(" mut");
            }
            for reason in &facts.reasons {
                let _ = write!(out, " !{}", reason.as_str());
            }
            if let Some(carriers) = &facts.carriers {
                out.push_str(" =[");
                for (index, carrier) in carriers.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{carrier:?}");
                }
                out.push(']');
            }
            out.push('\n');
        }
        for (a, b) in &self.unions {
            let _ = writeln!(out, "union {a} = {b}");
        }
        for edge in &self.edges {
            let kind = match edge.kind {
                EdgeKind::Value { source_dead: true } => "value(dead-source)",
                EdgeKind::Value { source_dead: false } => "value",
                EdgeKind::Shared => "shared",
                EdgeKind::HostPin => "host-pin",
                EdgeKind::HostSink { .. } => "host-sink",
                EdgeKind::Export => "export",
            };
            let _ = writeln!(out, "edge {} -{kind}- {}", edge.a, edge.b);
        }
        out
    }
}

/// A deterministic union-find over dense indices; the representative is the smallest index.
pub struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    pub fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    pub fn root(&mut self, mut index: usize) -> usize {
        while self.parent[index] != index {
            self.parent[index] = self.parent[self.parent[index]];
            index = self.parent[index];
        }
        index
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.root(a), self.root(b));
        match a.cmp(&b) {
            std::cmp::Ordering::Less => self.parent[b] = a,
            std::cmp::Ordering::Greater => self.parent[a] = b,
            std::cmp::Ordering::Equal => {}
        }
    }
}

/// Collects the value graph of the reachable program.
///
/// `funcs` is phase A's output; `interns` maps each item to the interns of the object it was read
/// from (the caller builds it from `LinkJir::interns`); `live` is the reachable set — the item
/// table holds every item of every object, and a boundary only exists in the program the walk
/// found, so an unreachable item's references must not pin anything and an unreachable vtable
/// must not group anything. Analysis only: nothing is rewritten.
pub fn collect(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &Interns>,
    live: &BTreeSet<JsName>,
) -> ValueGraph {
    let lowered: BTreeMap<ItemId, JsName> = funcs
        .iter()
        .filter_map(|(name, func)| Some((func.as_ref().ok()?.item, name.clone())))
        .collect();
    let mut collector = Collector {
        graph: ValueGraph::default(),
        table,
        funcs,
        interns,
        live,
        lowered,
        walk_cache: BTreeMap::new(),
        pinned_groups: BTreeSet::new(),
        vtable_targets: BTreeSet::new(),
        tame_box_views: BTreeSet::new(),
        consumed_box_fields: BTreeSet::new(),
    };
    collector
        .graph
        .nodes
        .insert(NodeKey::Baseline, NodeFacts::default());

    // The non-call reference scan, shared with the flat-ABI plan.
    let mut scan = RefScan::default();
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        let types = collector.types_of(name);
        for block in &func.blocks {
            for op in &block.ops {
                refs::scan_op(func, op, types, &mut scan);
            }
        }
    }

    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        collector.walk_cache.clear();
        collector.collect_func(name, func);
    }

    collector.vtable_groups();
    collector.fn_pointer_groups(&scan);
    collector.exports();
    collector.pins(&scan);
    collector.allocator();
    collector.graph
}

/// The path, family, type, and inherited legality facts of one discovered candidate position.
#[derive(Clone, Debug)]
struct PathInfo {
    path: Vec<Step>,
    kind: CandidateKind,
    ty: TypeId,
    reasons: BTreeSet<InfReason>,
    /// The type itself carries a pinned host boundary: the node gets a pin edge, not a unary
    /// reason (D2's host correction).
    host_pin: bool,
}

/// Which side of a call boundary the far end of an edge is.
enum FarSide<'a> {
    /// A lowered callee: nodes anchor on its own parameter values and return slot.
    Lowered(&'a JsName, &'a Func),
    /// No lowered body: an opaque boundary node carrying `reason`.
    Opaque(String, InfReason),
    /// An indirect-dispatch group.
    Group(GroupKey),
}

struct Collector<'a> {
    graph: ValueGraph,
    table: &'a BTreeMap<JsName, LinkItem>,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    interns: &'a BTreeMap<JsName, &'a Interns>,
    /// The reachable set: the only items whose boundaries exist in the program.
    live: &'a BTreeSet<JsName>,
    lowered: BTreeMap<ItemId, JsName>,
    walk_cache: BTreeMap<TypeId, Vec<PathInfo>>,
    /// Groups whose target set cannot be proven complete: their slots pin.
    pinned_groups: BTreeSet<GroupKey>,
    /// Items reached through a vtable method slot: modeled as group edges, not refs pins.
    vtable_targets: BTreeSet<ItemId>,
    /// The current function's whitelisted fresh-allocation box views (Phase 5, W5f): the
    /// results of `Transmute` casts [`tame_fresh_box_views`](Self::tame_fresh_box_views)
    /// admitted. The `Cast` arm skips its `RawProvenance` mark for exactly these results;
    /// cleared at the top of every `collect_func`.
    tame_box_views: BTreeSet<ValueId>,
    /// Canonical Box values consumed only by extracting transparent field zero. They have no
    /// later drop or observation, so NativeBox's shape-preserving ownership label can keep that
    /// projection legal.
    consumed_box_fields: BTreeSet<ValueId>,
}

impl<'a> Collector<'a> {
    fn types_of(&self, name: &JsName) -> Option<&'a TypeTable> {
        self.interns.get(name).map(|interns| &interns.types)
    }

    // -- discovery ----------------------------------------------------------------------------

    /// The candidate positions of `ty`, cached per item (the cache is cleared when the current
    /// type table changes).
    fn walk(&mut self, ty: TypeId, types: &TypeTable) -> Vec<PathInfo> {
        if let Some(cached) = self.walk_cache.get(&ty) {
            return cached.clone();
        }
        let mut out = Vec::new();
        walk_type(
            ty,
            types,
            &mut Vec::new(),
            &BTreeSet::new(),
            false,
            &mut out,
            &mut BTreeSet::new(),
        );
        self.walk_cache.insert(ty, out.clone());
        out
    }

    /// Ensures the nodes of one value, merging facts, and attaches the pin edge of a
    /// host-pinned type.
    fn ensure(&mut self, item: &JsName, site: Site, ty: TypeId, types: &TypeTable) {
        for info in self.walk(ty, types) {
            let key = NodeKey::Value {
                item: item.clone(),
                site,
                path: info.path.clone(),
            };
            self.ensure_key(&key, &info);
        }
    }

    fn ensure_key(&mut self, key: &NodeKey, info: &PathInfo) {
        let facts = self.graph.nodes.entry(key.clone()).or_default();
        facts.kind = facts.kind.or(Some(info.kind));
        facts.ty = facts.ty.or(Some(info.ty));
        facts.reasons.extend(info.reasons.iter().copied());
        if info.host_pin {
            self.graph.edges.insert(Edge {
                a: key.clone(),
                b: NodeKey::Baseline,
                kind: EdgeKind::HostPin,
            });
        }
    }

    /// Marks every node of `site` under `prefix` (all of them for `None`) with `reason`.
    fn mark(
        &mut self,
        item: &JsName,
        site: Site,
        ty: TypeId,
        types: &TypeTable,
        prefix: Option<&[Step]>,
        reason: InfReason,
    ) {
        for info in self.walk(ty, types) {
            if prefix.is_some_and(|prefix| !info.path.starts_with(prefix)) {
                continue;
            }
            let key = NodeKey::Value {
                item: item.clone(),
                site,
                path: info.path.clone(),
            };
            self.ensure_key(&key, &info);
            self.graph
                .nodes
                .get_mut(&key)
                .expect("the node was just ensured")
                .reasons
                .insert(reason);
        }
    }

    /// Marks every node of `site` under `prefix` as mutated.
    fn mark_mut(
        &mut self,
        item: &JsName,
        site: Site,
        ty: TypeId,
        types: &TypeTable,
        prefix: Option<&[Step]>,
    ) {
        for info in self.walk(ty, types) {
            if prefix.is_some_and(|prefix| !info.path.starts_with(prefix)) {
                continue;
            }
            let key = NodeKey::Value {
                item: item.clone(),
                site,
                path: info.path.clone(),
            };
            self.ensure_key(&key, &info);
            self.graph
                .nodes
                .get_mut(&key)
                .expect("the node was just ensured")
                .mutable = true;
        }
    }

    /// Unions the candidate positions of `(a, a_prefix)` and `(b, b_prefix)` by matching path
    /// suffixes. A candidate on one side with no counterpart on the other flowed into something
    /// discovery does not walk, and is pinned in place rather than silently disconnected.
    #[allow(clippy::too_many_arguments)]
    fn union_suffixes(
        &mut self,
        item: &JsName,
        a: Site,
        a_ty: TypeId,
        a_prefix: &[Step],
        b: Site,
        b_ty: TypeId,
        b_prefix: &[Step],
        types: &TypeTable,
    ) {
        self.union_suffixes_inner(item, a, a_ty, a_prefix, b, b_ty, b_prefix, types, true);
    }

    /// [`Self::union_suffixes`] for a REGISTRY-DESCRIBED flow (a standard operation's slot flow,
    /// at its summarized body or at a cut call site): only matching suffix pairs union, and a
    /// one-sided candidate is left free instead of pinned — the "projection inside a boundary"
    /// the miss rule guards against is here the operation's own contract (a `Vec` receiver whose
    /// element position has no candidate node is not an unmodeled read; the registry body owns
    /// the elements). This is what un-poisons the op's own receiver slot.
    #[allow(clippy::too_many_arguments)]
    fn flow_suffixes(
        &mut self,
        item: &JsName,
        a: Site,
        a_ty: TypeId,
        a_prefix: &[Step],
        b: Site,
        b_ty: TypeId,
        b_prefix: &[Step],
        types: &TypeTable,
    ) {
        self.union_suffixes_inner(item, a, a_ty, a_prefix, b, b_ty, b_prefix, types, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn union_suffixes_inner(
        &mut self,
        item: &JsName,
        a: Site,
        a_ty: TypeId,
        a_prefix: &[Step],
        b: Site,
        b_ty: TypeId,
        b_prefix: &[Step],
        types: &TypeTable,
        pin_misses: bool,
    ) {
        self.ensure(item, a, a_ty, types);
        self.ensure(item, b, b_ty, types);
        let wa = self.walk(a_ty, types);
        let wb = self.walk(b_ty, types);
        let a_paths: BTreeSet<&[Step]> = wa.iter().map(|info| info.path.as_slice()).collect();
        let b_paths: BTreeSet<&[Step]> = wb.iter().map(|info| info.path.as_slice()).collect();
        let mut unions = Vec::new();
        let mut misses = Vec::new();
        for info in &wa {
            let Some(suffix) = info.path.strip_prefix(a_prefix) else {
                // A candidate whose path is a proper prefix of the flow's is being projected
                // INSIDE its boundary — `String::as_bytes` reading through the struct — which
                // discovery does not model: the candidate itself is pinned in place.
                let consumed_box_field = info.kind == CandidateKind::Box
                    && info.path.is_empty()
                    && matches!(a, Site::Value(value) if self.consumed_box_fields.contains(&value));
                if !consumed_box_field
                    && a_prefix
                        .strip_prefix(info.path.as_slice())
                        .is_some_and(|rest| !rest.is_empty())
                {
                    misses.push((a, a_ty, info.path.clone()));
                }
                continue;
            };
            let mut far = b_prefix.to_vec();
            far.extend_from_slice(suffix);
            if b_paths.contains(far.as_slice()) {
                unions.push((
                    NodeKey::Value {
                        item: item.clone(),
                        site: a,
                        path: info.path.clone(),
                    },
                    NodeKey::Value {
                        item: item.clone(),
                        site: b,
                        path: far,
                    },
                ));
            } else {
                misses.push((a, a_ty, info.path.clone()));
            }
        }
        for info in &wb {
            let Some(suffix) = info.path.strip_prefix(b_prefix) else {
                if b_prefix
                    .strip_prefix(info.path.as_slice())
                    .is_some_and(|rest| !rest.is_empty())
                {
                    misses.push((b, b_ty, info.path.clone()));
                }
                continue;
            };
            let mut near = a_prefix.to_vec();
            near.extend_from_slice(suffix);
            if !a_paths.contains(near.as_slice()) {
                misses.push((b, b_ty, info.path.clone()));
            }
        }
        for (x, y) in unions {
            let pair = if x <= y { (x, y) } else { (y, x) };
            if pair.0 != pair.1 {
                self.graph.unions.insert(pair);
            }
        }
        if pin_misses {
            for (site, ty, path) in misses {
                self.mark(
                    item,
                    site,
                    ty,
                    types,
                    Some(&path),
                    InfReason::UnsupportedOperation,
                );
            }
        }
    }

    // -- one function -------------------------------------------------------------------------

    /// The Phase-5 (W5f) raw-provenance whitelist, written from the FingerprintJS autopsy: the
    /// one shape both ports' `legality.raw-provenance` candidates share is the inlined
    /// `Rc::new`/`Box::new` construction — a fresh allocation call whose raw result is
    /// transmuted (possibly more than once) into one candidate `Box` type, initialized through
    /// raw reborrows of candidate-free pointees, and never otherwise observed.
    ///
    /// Such a transmute is a rename, not an observation: the pointer's every use is enumerated
    /// here, and each admitted use either re-forms a view of the same fresh storage (`Copy`,
    /// `Move`, raw-to-raw casts, `BorrowView`/`RawView` reborrows) or moves candidate-free data
    /// through it (`Deref` reads, `StoreThrough` writes). All box views of one allocation union
    /// into one node, so aliasing between them stays visible to the planner. Anything else — an
    /// address cast, a pointer comparison or offset, a call, an aggregate, a terminator use, a
    /// second target type, a candidate-typed read or write — refuses the whole allocation and
    /// leaves today's `RawProvenance` pin in place. Freshness comes from the producer: only the
    /// alloc-crate allocation entry points qualify, so the pointer can never be a view into a
    /// candidate that already has other names (`as_ptr` and friends stay pinned).
    ///
    /// Returns the admitted view groups as `(transmute result, box type)` lists — the box
    /// types may differ across one group (`Box<MaybeUninit<T>>` for the init write beside the
    /// final `Box<T>` is the shape `Rc::new` leaves), which is why the union below carries each
    /// side's own type. The `Cast` arm consults [`Self::tame_box_views`] to skip its mark for
    /// exactly these results.
    fn tame_fresh_box_views(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
    ) -> Vec<Vec<(ValueId, TypeId)>> {
        // Identity by lowered name: the alloc-crate entry points that return a pointer to fresh
        // storage. A name not in this list is refused, which is the sound direction.
        fn fresh_allocation_callee(name: &str) -> bool {
            [
                "alloc$alloc$exchange_malloc$h",
                "alloc$boxed$box_new_uninit$h",
                "alloc$alloc$alloc$h",
                "alloc$alloc$alloc_zeroed$h",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        }
        let raw_pointer = |ty: TypeId| {
            matches!(
                types.get(types.peeled(ty)).map(|d| &d.kind),
                Some(TypeKind::RawPointer { .. })
            )
        };

        // Use lists (op, result) per value, and every terminator-carried value.
        let mut use_ops: BTreeMap<ValueId, Vec<(&OpKind, Option<ValueId>)>> = BTreeMap::new();
        let mut term_used: BTreeSet<ValueId> = BTreeSet::new();
        let mut producers: Vec<ValueId> = Vec::new();
        for block in &cx.func.blocks {
            for op in &block.ops {
                for used in op.kind.uses() {
                    use_ops.entry(used).or_default().push((&op.kind, op.result));
                }
                if let (
                    Some(result),
                    OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        semantic: CallSemantic::RawParts,
                        ..
                    },
                ) = (op.result, &op.kind)
                {
                    if self
                        .lowered
                        .get(item)
                        .is_some_and(|callee| fresh_allocation_callee(callee.as_str()))
                        && raw_pointer(cx.ty(result))
                    {
                        producers.push(result);
                    }
                }
            }
            term_used.extend(block.term.uses());
        }

        let mut groups = Vec::new();
        'producer: for p in producers {
            let mut views: Vec<(ValueId, TypeId)> = Vec::new();
            let mut seen: BTreeSet<ValueId> = BTreeSet::new();
            seen.insert(p);
            let mut stack = vec![p];
            while let Some(x) = stack.pop() {
                if term_used.contains(&x) {
                    continue 'producer;
                }
                for (kind, result) in use_ops.get(&x).map(Vec::as_slice).unwrap_or(&[]) {
                    match kind {
                        OpKind::Copy(_) | OpKind::Move(_) => {
                            let Some(result) = result else {
                                continue 'producer;
                            };
                            if seen.insert(*result) {
                                stack.push(*result);
                            }
                        }
                        OpKind::Cast {
                            kind: CastKind::Transmute,
                            to,
                            ..
                        } if {
                            let infos = self.walk(*to, types);
                            infos.len() == 1
                                && infos[0].path.is_empty()
                                && infos[0].kind == CandidateKind::Box
                        } =>
                        {
                            let Some(result) = result else {
                                continue 'producer;
                            };
                            views.push((*result, *to));
                        }
                        OpKind::Cast {
                            kind: CastKind::Transmute | CastKind::PtrToPtr,
                            to,
                            ..
                        } if raw_pointer(*to) => {
                            let Some(result) = result else {
                                continue 'producer;
                            };
                            if seen.insert(*result) {
                                stack.push(*result);
                            }
                        }
                        OpKind::BorrowView { root, path, .. }
                        | OpKind::RawView { root, path, .. }
                            if *root == Root::Value(x)
                                && !path
                                    .iter()
                                    .any(|proj| matches!(proj, Proj::Index(v) if *v == x)) =>
                        {
                            let Some(result) = result else {
                                continue 'producer;
                            };
                            if !self.walk(cx.ty(*result), types).is_empty() {
                                continue 'producer;
                            }
                            if seen.insert(*result) {
                                stack.push(*result);
                            }
                        }
                        OpKind::Deref(_) => {
                            if result
                                .is_none_or(|result| !self.walk(cx.ty(result), types).is_empty())
                            {
                                continue 'producer;
                            }
                        }
                        OpKind::StoreThrough { ptr, v } => {
                            if *ptr != x
                                || seen.contains(v)
                                || !self.walk(cx.ty(*v), types).is_empty()
                            {
                                continue 'producer;
                            }
                        }
                        _ => continue 'producer,
                    }
                }
            }
            if !views.is_empty() {
                groups.push(views);
            }
        }
        groups
    }

    /// Box values whose complete use list is one or more projections of transparent field zero.
    /// With no terminator, call, drop, borrow, copy, or second projection use, MIR has moved the
    /// owner out (the `Box::into_raw` shape used by `Rc::new`). NativeBox keeps that field's
    /// baseline pointer spelling, and no allocator-release decision remains for this value.
    fn consumed_box_fields(&mut self, cx: &FuncCx<'_>, types: &TypeTable) -> BTreeSet<ValueId> {
        let mut uses: BTreeMap<ValueId, Vec<(&OpKind, Option<ValueId>)>> = BTreeMap::new();
        let mut term_used = BTreeSet::new();
        for block in &cx.func.blocks {
            for op in &block.ops {
                for used in op.kind.uses() {
                    uses.entry(used).or_default().push((&op.kind, op.result));
                }
            }
            term_used.extend(block.term.uses());
        }
        (0..cx.func.values.len())
            .map(|index| ValueId(index as u32))
            .filter(|value| !term_used.contains(value))
            .filter(|value| {
                let infos = self.walk(cx.ty(*value), types);
                infos.len() == 1 && infos[0].path.is_empty() && infos[0].kind == CandidateKind::Box
            })
            .filter(|value| {
                let Some(uses) = uses.get(value) else {
                    return false;
                };
                !uses.is_empty() && uses.iter().all(|(kind, result)| {
                    matches!(kind, OpKind::Field { base, variant: None, field: 0 } if base == value)
                        && result.is_some()
                })
            })
            .collect()
    }

    fn collect_func(&mut self, name: &JsName, func: &Func) {
        let Some(types) = self.types_of(name) else {
            return;
        };
        let semantic = self
            .table
            .get(name)
            .and_then(|item| item.jir.body.as_ref())
            .map(|body| body.semantic);
        match semantic {
            Some(BodySemantic::Standard(
                StandardOperation::SliceSort | StandardOperation::SliceSortCmp,
            )) => {}
            Some(BodySemantic::Standard(operation)) => {
                self.summarize_standard(name, func, types, operation);
                return;
            }
            Some(BodySemantic::DropGlue(dropped))
                if self.summarize_drop_glue(name, func, types, dropped) =>
            {
                return;
            }
            Some(sem) if crate::lower::bstr::enabled() => {
                let bstr_op = self.table.get(name).and_then(|item| {
                    let interns = self.interns.get(name).copied()?;
                    let (crate_name, item_path) = item_origin_info(item, interns);
                    let funcs = self.funcs;
                    let table = self.table;
                    let interns_map = &self.interns;
                    let lowered = &self.lowered;
                    classify_bstr_op(
                        sem,
                        func,
                        types,
                        interns,
                        crate_name,
                        item_path,
                        |item| {
                            lowered
                                .get(&item)
                                .and_then(|cn| funcs.get(cn)?.as_ref().ok())
                        },
                        |sub_item| {
                            let cn = lowered.get(&sub_item)?;
                            let sub = table.get(cn)?;
                            let si = interns_map.get(cn).copied()?;
                            Some(item_origin_info(sub, si))
                        },
                    )
                });
                if let Some(op) = bstr_op {
                    self.summarize_slot_reqs(name, func, types, bstr_spec(op).slots);
                    return;
                }
            }
            _ => {}
        }

        // Definition sites, use counts, and definition blocks, for the dead-source rule.
        let mut uses = vec![0u32; func.values.len()];
        let mut defs: Vec<Option<&OpKind>> = vec![None; func.values.len()];
        let mut def_block: Vec<Option<u32>> = vec![None; func.values.len()];
        for block in &func.blocks {
            for param in &block.params {
                def_block[param.0 as usize] = Some(block.id.0);
            }
            for op in &block.ops {
                for used in op.kind.uses() {
                    uses[used.0 as usize] += 1;
                }
                if let Some(result) = op.result {
                    defs[result.0 as usize] = Some(&op.kind);
                    def_block[result.0 as usize] = Some(block.id.0);
                }
            }
            for used in block.term.uses() {
                uses[used.0 as usize] += 1;
            }
        }
        let cx = FuncCx {
            name,
            func,
            uses,
            defs,
            def_block,
        };

        // Discover every value up front, exactly as v1 discovered every local: an untouched
        // candidate is still a node the report counts.
        for (index, def) in func.values.iter().enumerate() {
            self.ensure(name, Site::Value(ValueId(index as u32)), def.ty, types);
        }
        for (index, cell) in func.cells.iter().enumerate() {
            self.ensure(name, Site::Cell(CellId(index as u32)), cell.ty, types);
        }
        self.ensure(name, Site::Ret, func.sig.result, types);

        // The W5f raw-provenance whitelist: fresh-allocation box views proven observation-free
        // union into one node and skip the `Transmute` pin below.
        self.tame_box_views.clear();
        self.consumed_box_fields = self.consumed_box_fields(&cx, types);
        for views in self.tame_fresh_box_views(&cx, types) {
            self.tame_box_views
                .extend(views.iter().map(|(view, _)| *view));
            for pair in views.windows(2) {
                self.union_suffixes(
                    name,
                    Site::Value(pair[0].0),
                    pair[0].1,
                    &[],
                    Site::Value(pair[1].0),
                    pair[1].1,
                    &[],
                    types,
                );
            }
        }

        let inlined_string_vec = if crate::lower::bstr::enabled() {
            self.interns.get(name).copied().and_then(|interns| {
                analyze_inlined_string_vec(func, types, interns, |item, _| {
                    is_raw_vec_reserve_in_table(self.table, self.interns, &self.lowered, item)
                })
            })
        } else {
            None
        };
        if let Some(ref analysis) = inlined_string_vec {
            for cluster in &analysis.clusters {
                let (root_site, root_ty, prefix): (Site, TypeId, &[Step]) = match cluster.root {
                    StringRoot::Cell(c) => (Site::Cell(c), cluster.str_ty, &[]),
                    StringRoot::Value(v) => {
                        if cluster.root_is_ref {
                            (Site::Value(v), cx.ty(v), &[Step::Deref])
                        } else {
                            (Site::Value(v), cluster.str_ty, &[])
                        }
                    }
                };
                self.ensure(name, root_site, root_ty, types);
                if cluster.is_mut {
                    self.mark_mut(name, root_site, root_ty, types, Some(prefix));
                    self.constrain(
                        name,
                        root_site,
                        root_ty,
                        types,
                        prefix,
                        &[Carrier::StringBuilder],
                    );
                } else {
                    self.constrain(
                        name,
                        root_site,
                        root_ty,
                        types,
                        prefix,
                        &[Carrier::HostString, Carrier::StringBuilder],
                    );
                }
            }
        }

        for (bi, block) in func.blocks.iter().enumerate() {
            for (oi, op) in block.ops.iter().enumerate() {
                if inlined_string_vec
                    .as_ref()
                    .is_some_and(|a| a.op_cluster.contains_key(&(bi, oi)))
                {
                    continue;
                }
                self.collect_op(&cx, types, block.id.0, op.result, &op.kind);
            }
            match &block.term {
                RTerm::Return(Some(v)) => {
                    self.union_suffixes(
                        name,
                        Site::Value(*v),
                        cx.ty(*v),
                        &[],
                        Site::Ret,
                        func.sig.result,
                        &[],
                        types,
                    );
                }
                RTerm::Throw(Some(v)) => {
                    // A thrown payload is observed by whatever catches it, which nothing models.
                    self.mark(
                        name,
                        Site::Value(*v),
                        cx.ty(*v),
                        types,
                        None,
                        InfReason::UnsupportedOperation,
                    );
                }
                term => {
                    for (target, args) in term.edges() {
                        let params = &func.blocks[target.0 as usize].params;
                        for (arg, param) in args.iter().zip(params) {
                            self.union_suffixes(
                                name,
                                Site::Value(*arg),
                                cx.ty(*arg),
                                &[],
                                Site::Value(*param),
                                cx.ty(*param),
                                &[],
                                types,
                            );
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_op(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
        block: u32,
        result: Option<ValueId>,
        kind: &OpKind,
    ) {
        let name = cx.name;
        match kind {
            OpKind::Const(tree) => {
                if let Some(result) = result {
                    self.const_statics(cx, types, result, tree);
                }
            }
            OpKind::Copy(v) | OpKind::Move(v) => {
                if let Some(result) = result {
                    self.union_suffixes(
                        name,
                        Site::Value(*v),
                        cx.ty(*v),
                        &[],
                        Site::Value(result),
                        cx.ty(result),
                        &[],
                        types,
                    );
                }
            }
            OpKind::Aggregate {
                ty,
                variant,
                active_field,
                fields,
            } => {
                let Some(result) = result else { return };
                if active_field.is_some() {
                    // A union aggregate: fields overlay, which the path model does not spell.
                    for field in fields {
                        self.mark(
                            name,
                            Site::Value(*field),
                            cx.ty(*field),
                            types,
                            None,
                            InfReason::UnsupportedOperation,
                        );
                    }
                    return;
                }
                for (index, field) in fields.iter().enumerate() {
                    let prefix = aggregate_prefix(types, *ty, *variant, index as u32);
                    match prefix {
                        Some(prefix) => self.union_suffixes(
                            name,
                            Site::Value(*field),
                            cx.ty(*field),
                            &[],
                            Site::Value(result),
                            cx.ty(result),
                            &prefix,
                            types,
                        ),
                        // The destination type is not walked (an array, a coroutine, ...): pin
                        // the candidate that flowed in, where it still has a name.
                        None => self.mark(
                            name,
                            Site::Value(*field),
                            cx.ty(*field),
                            types,
                            None,
                            InfReason::UnsupportedOperation,
                        ),
                    }
                }
            }
            OpKind::Repeat { v, .. } => {
                self.mark(
                    name,
                    Site::Value(*v),
                    cx.ty(*v),
                    types,
                    None,
                    InfReason::UnsupportedOperation,
                );
            }
            OpKind::Field {
                base,
                variant,
                field,
            } => {
                let Some(result) = result else { return };
                let mut prefix = Vec::new();
                if let Some(variant) = variant {
                    prefix.push(Step::Variant(*variant));
                }
                prefix.push(Step::Field(*field));
                self.union_suffixes(
                    name,
                    Site::Value(*base),
                    cx.ty(*base),
                    &prefix,
                    Site::Value(result),
                    cx.ty(result),
                    &[],
                    types,
                );
            }
            OpKind::SetField {
                base,
                variant,
                field,
                v,
            } => {
                let mut prefix = Vec::new();
                if let Some(variant) = variant {
                    prefix.push(Step::Variant(*variant));
                }
                prefix.push(Step::Field(*field));
                self.union_suffixes(
                    name,
                    Site::Value(*v),
                    cx.ty(*v),
                    &[],
                    Site::Value(*base),
                    cx.ty(*base),
                    &prefix,
                    types,
                );
                self.mark_mut(name, Site::Value(*base), cx.ty(*base), types, Some(&prefix));
            }
            OpKind::Discriminant(_) => {}
            OpKind::SetDiscriminant { base, .. } => {
                self.mark_mut(name, Site::Value(*base), cx.ty(*base), types, None);
            }
            OpKind::CellAlloc(_) => {}
            OpKind::CellLoad(cell) => {
                if let Some(result) = result {
                    self.union_suffixes(
                        name,
                        Site::Cell(*cell),
                        cx.cell_ty(*cell),
                        &[],
                        Site::Value(result),
                        cx.ty(result),
                        &[],
                        types,
                    );
                }
            }
            OpKind::CellStore { cell, v } => {
                self.union_suffixes(
                    name,
                    Site::Value(*v),
                    cx.ty(*v),
                    &[],
                    Site::Cell(*cell),
                    cx.cell_ty(*cell),
                    &[],
                    types,
                );
                self.mark_mut(name, Site::Cell(*cell), cx.cell_ty(*cell), types, None);
            }
            OpKind::BorrowView {
                root,
                path,
                mutable,
            } => {
                let (site, root_ty) = cx.root(*root);
                match (proj_steps(path), result) {
                    (Some(steps), Some(result)) => {
                        self.union_suffixes(
                            name,
                            site,
                            root_ty,
                            &steps,
                            Site::Value(result),
                            cx.ty(result),
                            &[Step::Deref],
                            types,
                        );
                        if *mutable {
                            self.mark_mut(name, site, root_ty, types, Some(&steps));
                        }
                    }
                    _ => {
                        self.mark(
                            name,
                            site,
                            root_ty,
                            types,
                            None,
                            InfReason::UnsupportedOperation,
                        );
                        if *mutable {
                            self.mark_mut(name, site, root_ty, types, None);
                        }
                    }
                }
            }
            OpKind::RawView { root, path, .. } => {
                let (site, root_ty) = cx.root(*root);
                let prefix = proj_steps(path);
                self.mark(
                    name,
                    site,
                    root_ty,
                    types,
                    prefix.as_deref(),
                    InfReason::RawProvenance,
                );
                if let Some(result) = result {
                    self.mark(
                        name,
                        Site::Value(result),
                        cx.ty(result),
                        types,
                        None,
                        InfReason::RawProvenance,
                    );
                }
            }
            OpKind::SliceView { ptr, .. } => {
                let Some(result) = result else { return };
                let slice_pointee = matches!(
                    types
                        .get(types.peeled(cx.ty(*ptr)))
                        .map(|descriptor| &descriptor.kind),
                    Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. })
                        if matches!(
                            types.get(types.peeled(*pointee)).map(|d| &d.kind),
                            Some(TypeKind::Slice(_))
                        )
                );
                if slice_pointee {
                    self.union_suffixes(
                        name,
                        Site::Value(*ptr),
                        cx.ty(*ptr),
                        &[Step::Deref],
                        Site::Value(result),
                        cx.ty(result),
                        &[Step::Deref],
                        types,
                    );
                } else {
                    self.mark(
                        name,
                        Site::Value(*ptr),
                        cx.ty(*ptr),
                        types,
                        None,
                        InfReason::RawProvenance,
                    );
                    self.mark(
                        name,
                        Site::Value(result),
                        cx.ty(result),
                        types,
                        None,
                        InfReason::RawProvenance,
                    );
                }
            }
            OpKind::Deref(ptr) => {
                if let Some(result) = result {
                    self.union_suffixes(
                        name,
                        Site::Value(*ptr),
                        cx.ty(*ptr),
                        &[Step::Deref],
                        Site::Value(result),
                        cx.ty(result),
                        &[],
                        types,
                    );
                }
            }
            OpKind::StoreThrough { ptr, v } => {
                self.union_suffixes(
                    name,
                    Site::Value(*v),
                    cx.ty(*v),
                    &[],
                    Site::Value(*ptr),
                    cx.ty(*ptr),
                    &[Step::Deref],
                    types,
                );
                self.mark_mut(
                    name,
                    Site::Value(*ptr),
                    cx.ty(*ptr),
                    types,
                    Some(&[Step::Deref]),
                );
            }
            OpKind::Binary { .. } | OpKind::Unary { .. } => {}
            OpKind::Cast { kind, v, to, .. } => match kind {
                CastKind::PointerExposeProvenance
                | CastKind::PointerWithExposedProvenance
                | CastKind::PtrToPtr
                | CastKind::FnPtrToPtr
                | CastKind::Transmute => {
                    // The W5f whitelist: an admitted fresh-allocation box view is a rename of
                    // storage nothing else names — the pre-pass proved every use of the source
                    // pointer and unioned the sibling views, so neither side pins.
                    if matches!(kind, CastKind::Transmute)
                        && result.is_some_and(|result| self.tame_box_views.contains(&result))
                    {
                        return;
                    }
                    self.mark(
                        name,
                        Site::Value(*v),
                        cx.ty(*v),
                        types,
                        None,
                        InfReason::RawProvenance,
                    );
                    if let Some(result) = result {
                        self.mark(
                            name,
                            Site::Value(result),
                            cx.ty(result),
                            types,
                            None,
                            InfReason::RawProvenance,
                        );
                    }
                }
                CastKind::PointerCoercion | CastKind::Subtype => {
                    if coerces_to_dynamic(types, *to) {
                        // The value escapes behind a `dyn`: its storage is dispatched with
                        // carriers no call edge models.
                        self.mark(
                            name,
                            Site::Value(*v),
                            cx.ty(*v),
                            types,
                            None,
                            InfReason::UnknownCall,
                        );
                    } else if let Some(result) = result {
                        self.union_suffixes(
                            name,
                            Site::Value(*v),
                            cx.ty(*v),
                            &[],
                            Site::Value(result),
                            cx.ty(result),
                            &[],
                            types,
                        );
                    }
                }
                CastKind::IntToInt
                | CastKind::FloatToInt
                | CastKind::FloatToFloat
                | CastKind::IntToFloat => {}
            },
            OpKind::Call {
                callee,
                args,
                semantic,
                host,
                ..
            } => self.collect_call(cx, types, block, result, callee, args, *semantic, host),
            OpKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.mark(
                        name,
                        Site::Value(*arg),
                        cx.ty(*arg),
                        types,
                        None,
                        InfReason::UnsupportedOperation,
                    );
                }
                if let Some(result) = result {
                    self.mark(
                        name,
                        Site::Value(result),
                        cx.ty(result),
                        types,
                        None,
                        InfReason::UnsupportedOperation,
                    );
                }
            }
            OpKind::Host { args, .. } => {
                // Reserved: carried like a host call classified by type alone.
                for arg in args {
                    self.host_slot_by_type(cx, types, *arg);
                }
                if let Some(result) = result {
                    self.host_slot_by_type(cx, types, result);
                }
            }
            OpKind::Drop { root, path, glue } => {
                let (site, root_ty) = cx.root(*root);
                match glue {
                    DropGlue::None => {}
                    DropGlue::Direct { item, .. } => match proj_steps(path) {
                        Some(steps) => self.drop_edge(cx, types, site, root_ty, &steps, *item),
                        None => self.mark(
                            name,
                            site,
                            root_ty,
                            types,
                            None,
                            InfReason::UnsupportedOperation,
                        ),
                    },
                    DropGlue::Dynamic => {
                        let prefix = proj_steps(path);
                        self.mark(
                            name,
                            site,
                            root_ty,
                            types,
                            prefix.as_deref(),
                            InfReason::UnknownCall,
                        );
                    }
                }
            }
            OpKind::Assert { .. } | OpKind::Zombie(_) => {}
            OpKind::Materialize(v) | OpKind::Convert { v, .. } => {
                if let Some(result) = result {
                    self.union_suffixes(
                        name,
                        Site::Value(*v),
                        cx.ty(*v),
                        &[],
                        Site::Value(result),
                        cx.ty(result),
                        &[],
                        types,
                    );
                }
            }
            OpKind::SpillRead(_) | OpKind::SpillWrite { .. } => {}
        }
    }

    /// Edges every static a constant tree references against the deref paths of the constant's
    /// value: the storage behind the pointer is the static's.
    fn const_statics(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
        result: ValueId,
        tree: &ConstTree,
    ) {
        let mut statics = Vec::new();
        collect_static_refs(tree, &mut statics);
        if statics.is_empty() {
            return;
        }
        let name = cx.name;
        self.ensure(name, Site::Value(result), cx.ty(result), types);
        for info in self.walk(cx.ty(result), types) {
            if !info.path.contains(&Step::Deref) {
                continue;
            }
            for definition in &statics {
                let far = NodeKey::Static {
                    definition: *definition,
                    path: info.path.clone(),
                };
                let facts = self.graph.nodes.entry(far.clone()).or_default();
                facts.kind = facts.kind.or(Some(info.kind));
                facts.ty = facts.ty.or(Some(info.ty));
                facts.reasons.insert(InfReason::StaticEscape);
                self.graph.edges.insert(Edge {
                    a: NodeKey::Value {
                        item: name.clone(),
                        site: Site::Value(result),
                        path: info.path.clone(),
                    },
                    b: far,
                    kind: EdgeKind::Shared,
                });
            }
        }
    }

    // -- calls --------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn collect_call(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
        block: u32,
        result: Option<ValueId>,
        callee: &Callee,
        args: &[ValueId],
        semantic: CallSemantic,
        host: &Option<HostPayload>,
    ) {
        let name = cx.name;
        match semantic {
            CallSemantic::ViewAbi => {
                for arg in args {
                    self.mark(
                        name,
                        Site::Value(*arg),
                        cx.ty(*arg),
                        types,
                        None,
                        InfReason::ViewAbi,
                    );
                }
                if let Some(result) = result {
                    self.mark(
                        name,
                        Site::Value(result),
                        cx.ty(result),
                        types,
                        None,
                        InfReason::ViewAbi,
                    );
                }
                return;
            }
            CallSemantic::RawParts | CallSemantic::RawJavaScript => {
                for arg in args {
                    self.mark(
                        name,
                        Site::Value(*arg),
                        cx.ty(*arg),
                        types,
                        None,
                        InfReason::RawProvenance,
                    );
                }
                if let Some(result) = result {
                    self.mark(
                        name,
                        Site::Value(result),
                        cx.ty(result),
                        types,
                        None,
                        InfReason::RawProvenance,
                    );
                }
                return;
            }
            CallSemantic::Host => {
                self.host_call(cx, types, args, result, host);
                return;
            }
            CallSemantic::GlobalAllocator | CallSemantic::Standard(_) | CallSemantic::Ordinary => {}
        }
        if host.is_some() {
            self.host_call(cx, types, args, result, host);
            return;
        }
        match callee {
            Callee::Direct { item, .. } => {
                let callee_name = self.lowered.get(item).cloned();
                if let Some(callee_name) = &callee_name {
                    self.graph
                        .direct_calls
                        .entry(name.clone())
                        .or_default()
                        .insert(callee_name.clone());
                }
                let funcs = self.funcs;
                let lowered = callee_name.as_ref().and_then(|callee_name| {
                    match funcs.get(callee_name) {
                        // A shape the positional pairing below cannot align is treated as an
                        // unseen body rather than mispaired. A `#[track_caller]` callee's one
                        // trailing `$loc` parameter still aligns: the location is never a
                        // candidate, and the Rust arguments pair positionally before it.
                        Some(Ok(func))
                            if func.sig.abi.spread.is_none()
                                && (func.params.len() == args.len()
                                    || (func.sig.abi.caller_location
                                        && func.params.len() == args.len() + 1)) =>
                        {
                            Some((callee_name, func))
                        }
                        _ => None,
                    }
                });
                match lowered {
                    Some((callee_name, callee_func)) => {
                        let cut = match semantic {
                            // D2's region cut: a registry-covered standard call never
                            // must-equal-joins the caller's storage into the callee's body.
                            CallSemantic::Standard(operation) => semantic_ops::registry(operation),
                            _ => None,
                        };
                        let bstr_cut = if cut.is_none() && crate::lower::bstr::enabled() {
                            let callee_item = self.table.get(callee_name);
                            let callee_sem = callee_item
                                .and_then(|item| item.jir.body.as_ref())
                                .map(|body| body.semantic);
                            if let (Some(callee_item), Some(sem), Some(callee_interns)) = (
                                callee_item,
                                callee_sem,
                                self.interns.get(callee_name).copied(),
                            ) {
                                let (crate_name, item_path) =
                                    item_origin_info(callee_item, callee_interns);
                                let lowered_map = &self.lowered;
                                let table = self.table;
                                let interns_map = &self.interns;
                                classify_bstr_op(
                                    sem,
                                    callee_func,
                                    &callee_interns.types,
                                    callee_interns,
                                    crate_name,
                                    item_path,
                                    |sub_item| {
                                        lowered_map
                                            .get(&sub_item)
                                            .and_then(|n| funcs.get(n)?.as_ref().ok())
                                    },
                                    |sub_item| {
                                        let cn = lowered_map.get(&sub_item)?;
                                        let sub = table.get(cn)?;
                                        let si = interns_map.get(cn).copied()?;
                                        Some(item_origin_info(sub, si))
                                    },
                                )
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        match cut {
                            Some(op) => {
                                // By-value paths keep their priced adapter edges (mixing stays
                                // legal there, exactly as before the cut); deref paths get no
                                // edge — the registry's slot requirements constrain the CALLER's
                                // own values instead, and `plan/representation.rs`'s asymmetric
                                // coherence arcs keep the callee's body honest without ever
                                // letting it poison the caller.
                                self.boundary_edges(
                                    cx,
                                    types,
                                    block,
                                    args,
                                    result,
                                    FarSide::Lowered(callee_name, callee_func),
                                    true,
                                );
                                let slots: Vec<(Site, TypeId)> = args
                                    .iter()
                                    .map(|arg| (Site::Value(*arg), cx.ty(*arg)))
                                    .collect();
                                let ret = result.map(|result| (Site::Value(result), cx.ty(result)));
                                self.standard_slots(name, op, &slots, ret, types);
                                self.standard_flows(name, op.id, &slots, ret, types);
                            }
                            None if bstr_cut.is_some() => {
                                let spec = bstr_spec(bstr_cut.unwrap());
                                self.boundary_edges(
                                    cx,
                                    types,
                                    block,
                                    args,
                                    result,
                                    FarSide::Lowered(callee_name, callee_func),
                                    true,
                                );
                                let slots: Vec<(Site, TypeId)> = args
                                    .iter()
                                    .map(|arg| (Site::Value(*arg), cx.ty(*arg)))
                                    .collect();
                                let ret = result.map(|result| (Site::Value(result), cx.ty(result)));
                                self.apply_slot_reqs(name, spec.slots, &slots, ret, types);
                            }
                            None => self.boundary_edges(
                                cx,
                                types,
                                block,
                                args,
                                result,
                                FarSide::Lowered(callee_name, callee_func),
                                false,
                            ),
                        }
                    }
                    None => {
                        let label = callee_name
                            .as_ref()
                            .map(|name| name.as_str().to_owned())
                            .unwrap_or_else(|| {
                                format!(
                                    "{:016x}.{:016x}",
                                    item.definition.crate_id, item.definition.local_hash
                                )
                            });
                        self.boundary_edges(
                            cx,
                            types,
                            block,
                            args,
                            result,
                            FarSide::Opaque(label, InfReason::BodyUnavailable),
                            false,
                        );
                        // The callee IS lowered, but this call could not be aligned with it, so
                        // its edges went to the opaque stand-in above. The callee's own slot
                        // nodes must not float free of that: an unmodeled caller pins them,
                        // exactly as `pins` does, or a plan could commit the body while this
                        // very call keeps passing baseline values.
                        if let Some(callee_name) = callee_name {
                            if let Some(Ok(callee_func)) = self.funcs.get(&callee_name) {
                                let slots: Vec<(Site, TypeId)> = callee_func
                                    .params
                                    .iter()
                                    .map(|param| {
                                        (
                                            Site::Value(*param),
                                            callee_func.values[param.0 as usize].ty,
                                        )
                                    })
                                    .chain(std::iter::once((Site::Ret, callee_func.sig.result)))
                                    .collect();
                                if let Some(callee_types) = self.types_of(&callee_name) {
                                    for (site, ty) in slots {
                                        self.mark(
                                            &callee_name,
                                            site,
                                            ty,
                                            callee_types,
                                            None,
                                            InfReason::UnknownCall,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Callee::Foreign { symbol, .. } => {
                let label = self
                    .interns
                    .get(name)
                    .and_then(|interns| interns.str(*symbol))
                    .unwrap_or("foreign")
                    .to_owned();
                self.boundary_edges(
                    cx,
                    types,
                    block,
                    args,
                    result,
                    FarSide::Opaque(label, InfReason::ForeignCall),
                    false,
                );
            }
            Callee::Indirect { signature } => {
                let peeled = types.peeled(*signature);
                if matches!(
                    types.get(peeled).map(|descriptor| &descriptor.kind),
                    Some(TypeKind::FnPtr { .. })
                ) {
                    let key = GroupKey::FnPtr(peeled);
                    self.pinned_groups.insert(key);
                    self.boundary_edges(cx, types, block, args, result, FarSide::Group(key), false);
                } else {
                    for arg in args {
                        self.mark(
                            name,
                            Site::Value(*arg),
                            cx.ty(*arg),
                            types,
                            None,
                            InfReason::UnknownCall,
                        );
                    }
                    if let Some(result) = result {
                        self.mark(
                            name,
                            Site::Value(result),
                            cx.ty(result),
                            types,
                            None,
                            InfReason::UnknownCall,
                        );
                    }
                }
            }
            Callee::Virtual { slot, receiver } => {
                let trait_ref = match types
                    .get(types.peeled(deref_pointee(types, *receiver)))
                    .map(|descriptor| &descriptor.kind)
                {
                    Some(TypeKind::Dynamic { principal, .. }) => *principal,
                    _ => None,
                };
                let key = GroupKey::Vtable {
                    trait_ref,
                    slot: *slot,
                };
                self.boundary_edges(cx, types, block, args, result, FarSide::Group(key), false);
            }
            Callee::Runtime { .. } => {
                for arg in args {
                    self.host_slot_by_type(cx, types, *arg);
                }
                if let Some(result) = result {
                    self.host_slot_by_type(cx, types, result);
                }
            }
            Callee::Intrinsic { .. } => {
                for arg in args {
                    self.mark(
                        name,
                        Site::Value(*arg),
                        cx.ty(*arg),
                        types,
                        None,
                        InfReason::UnsupportedOperation,
                    );
                }
                if let Some(result) = result {
                    self.mark(
                        name,
                        Site::Value(result),
                        cx.ty(result),
                        types,
                        None,
                        InfReason::UnsupportedOperation,
                    );
                }
            }
            Callee::Nop => {}
        }
    }

    /// The edges of one call boundary: each argument slot and the return slot, paired by path
    /// suffix. A path that crosses a `Deref` shares storage and must-equals; a by-value path is
    /// an adapter edge carrying the dead-source direction. With `value_paths_only` (the D2 cut),
    /// deref-crossing paths make no edge at all — the caller side carries the constraints.
    #[allow(clippy::too_many_arguments)]
    fn boundary_edges(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
        block: u32,
        args: &[ValueId],
        result: Option<ValueId>,
        far: FarSide<'_>,
        value_paths_only: bool,
    ) {
        let name = cx.name;
        for (index, arg) in args.iter().enumerate() {
            let source_dead = cx.dead_unaliased(*arg, block);
            let walk = self.walk(cx.ty(*arg), types);
            self.ensure(name, Site::Value(*arg), cx.ty(*arg), types);
            for info in walk {
                if value_paths_only && info.path.contains(&Step::Deref) {
                    continue;
                }
                let Some(b) = self.far_key(&far, BoundarySlot::Param(index as u32), &info) else {
                    continue;
                };
                let kind = if info.path.contains(&Step::Deref) {
                    EdgeKind::Shared
                } else {
                    EdgeKind::Value { source_dead }
                };
                self.graph.edges.insert(Edge {
                    a: NodeKey::Value {
                        item: name.clone(),
                        site: Site::Value(*arg),
                        path: info.path.clone(),
                    },
                    b,
                    kind,
                });
            }
        }
        if let Some(result) = result {
            let walk = self.walk(cx.ty(result), types);
            self.ensure(name, Site::Value(result), cx.ty(result), types);
            for info in walk {
                if value_paths_only && info.path.contains(&Step::Deref) {
                    continue;
                }
                let Some(a) = self.far_key(&far, BoundarySlot::Ret, &info) else {
                    continue;
                };
                let kind = if info.path.contains(&Step::Deref) {
                    EdgeKind::Shared
                } else {
                    // A returned value's aliasing is the callee's to prove; nothing does yet.
                    EdgeKind::Value { source_dead: false }
                };
                self.graph.edges.insert(Edge {
                    a,
                    b: NodeKey::Value {
                        item: name.clone(),
                        site: Site::Value(result),
                        path: info.path.clone(),
                    },
                    kind,
                });
            }
        }
    }

    /// The far-side node of one slot at one path, ensured. `None` skips the edge (a lowered
    /// callee without that parameter).
    fn far_key(
        &mut self,
        far: &FarSide<'_>,
        slot: BoundarySlot,
        info: &PathInfo,
    ) -> Option<NodeKey> {
        let key = match far {
            FarSide::Lowered(callee, func) => {
                let site = match slot {
                    BoundarySlot::Param(index) => Site::Value(*func.params.get(index as usize)?),
                    BoundarySlot::Ret => Site::Ret,
                };
                NodeKey::Value {
                    item: (*callee).clone(),
                    site,
                    path: info.path.clone(),
                }
            }
            FarSide::Opaque(item, reason) => {
                let key = NodeKey::Opaque {
                    item: item.clone(),
                    slot,
                    path: info.path.clone(),
                };
                self.ensure_key(&key, info);
                self.graph
                    .nodes
                    .get_mut(&key)
                    .expect("the node was just ensured")
                    .reasons
                    .insert(*reason);
                return Some(key);
            }
            FarSide::Group(group) => NodeKey::Group {
                key: *group,
                slot,
                path: info.path.clone(),
            },
        };
        self.ensure_key(&key, info);
        Some(key)
    }

    /// The `Drop` boundary: the dropped storage shares with the glue's pointer parameter.
    fn drop_edge(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
        site: Site,
        root_ty: TypeId,
        steps: &[Step],
        glue: ItemId,
    ) {
        let name = cx.name;
        self.ensure(name, site, root_ty, types);
        let funcs = self.funcs;
        let far =
            self.lowered
                .get(&glue)
                .cloned()
                .and_then(|glue_name| match funcs.get(&glue_name) {
                    Some(Ok(func)) => Some((glue_name, *func.params.first()?)),
                    _ => None,
                });
        if let Some((glue_name, _)) = &far {
            self.graph
                .direct_calls
                .entry(name.clone())
                .or_default()
                .insert(glue_name.clone());
        }
        for info in self.walk(root_ty, types) {
            let Some(suffix) = info.path.strip_prefix(steps) else {
                continue;
            };
            let mut far_path = vec![Step::Deref];
            far_path.extend_from_slice(suffix);
            let far_info = PathInfo {
                path: far_path.clone(),
                ..info.clone()
            };
            let b = match &far {
                Some((glue_name, param)) => {
                    let key = NodeKey::Value {
                        item: glue_name.clone(),
                        site: Site::Value(*param),
                        path: far_path,
                    };
                    self.ensure_key(&key, &far_info);
                    key
                }
                None => {
                    let key = NodeKey::Opaque {
                        item: format!(
                            "{:016x}.{:016x}",
                            glue.definition.crate_id, glue.definition.local_hash
                        ),
                        slot: BoundarySlot::Param(0),
                        path: far_path,
                    };
                    self.ensure_key(&key, &far_info);
                    self.graph
                        .nodes
                        .get_mut(&key)
                        .expect("the node was just ensured")
                        .reasons
                        .insert(InfReason::BodyUnavailable);
                    key
                }
            };
            // A drop is the value's last use by Rust's own rules — storage still aliased after
            // it would be use-after-free — so the edge is the adapter kind with a proven-dead
            // source, never must-equal: one pinned dropped value must not tie every other value
            // sharing the same glue to its label (the component poison D2 exists to kill). The
            // commit spells the cut as a `Convert` right before the `Drop` (W4b).
            self.graph.edges.insert(Edge {
                a: NodeKey::Value {
                    item: name.clone(),
                    site,
                    path: info.path.clone(),
                },
                b,
                kind: EdgeKind::Value { source_dead: true },
            });
        }
    }

    // -- host boundaries ----------------------------------------------------------------------

    /// One host call: every slot classified per its declared contract (V3 facts where the
    /// descriptor carries them, `HostParameterClass` where it does not), the term on the EDGE.
    fn host_call(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
        args: &[ValueId],
        result: Option<ValueId>,
        host: &Option<HostPayload>,
    ) {
        let descriptor = match host {
            Some(HostPayload::Extern(bytes)) => self
                .interns
                .get(cx.name)
                .and_then(|interns| interns.bytes.get(bytes.0 as usize))
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .and_then(|text| Descriptor::decode(text).ok()),
            _ => None,
        };
        let mut slots: Vec<(Slot, ValueId)> = args
            .iter()
            .enumerate()
            .map(|(index, arg)| (Slot::Parameter(index as u32), *arg))
            .collect();
        if let Some(result) = result {
            slots.push((Slot::Result, result));
        }
        for (slot, value) in slots {
            let (term, accepts) = match &descriptor {
                Some(descriptor) => {
                    let contract = HostContract::slot(descriptor, slot, cx.ty(value), types);
                    (contract.term, contract.accepts)
                }
                None => match HostParameterClass::of(cx.ty(value), types) {
                    HostParameterClass::Sink => (SlotTerm::Sink, SlotFacts::default()),
                    HostParameterClass::Pin => (SlotTerm::Pin, SlotFacts::default()),
                },
            };
            self.host_slot(cx, types, value, term, accepts);
        }
    }

    fn host_slot_by_type(&mut self, cx: &FuncCx<'_>, types: &TypeTable, value: ValueId) {
        let term = match HostParameterClass::of(cx.ty(value), types) {
            HostParameterClass::Sink => SlotTerm::Sink,
            HostParameterClass::Pin => SlotTerm::Pin,
        };
        self.host_slot(cx, types, value, term, SlotFacts::default());
    }

    fn host_slot(
        &mut self,
        cx: &FuncCx<'_>,
        types: &TypeTable,
        value: ValueId,
        term: SlotTerm,
        accepts: SlotFacts,
    ) {
        let kind = match term {
            SlotTerm::Sink => EdgeKind::HostSink { accepts },
            // An `owns` (Transfer) cut has no spelling: `commit_func` skips host calls, so a
            // `Convert` on a host edge would never be inserted, and a finitely priced cut here
            // would be a silent miscompile. Declined (YAGNI) until the commit can spell
            // host-edge converts; the slot pins its edge like any baseline-observing slot.
            SlotTerm::Pin | SlotTerm::Transfer => EdgeKind::HostPin,
        };
        self.ensure(cx.name, Site::Value(value), cx.ty(value), types);
        for info in self.walk(cx.ty(value), types) {
            self.graph.edges.insert(Edge {
                a: NodeKey::Value {
                    item: cx.name.clone(),
                    site: Site::Value(value),
                    path: info.path.clone(),
                },
                b: NodeKey::Baseline,
                kind,
            });
        }
    }

    // -- summaries ----------------------------------------------------------------------------

    /// A standard-operation body is summarized instead of collected: the raw internals of
    /// `Vec::push` never touch the graph, only the slot flows v1 recorded for the operation.
    fn summarize_standard(
        &mut self,
        name: &JsName,
        func: &Func,
        types: &TypeTable,
        operation: StandardOperation,
    ) {
        for param in &func.params {
            self.ensure(
                name,
                Site::Value(*param),
                func.values[param.0 as usize].ty,
                types,
            );
        }
        self.ensure(name, Site::Ret, func.sig.result, types);
        // Coverage is the registry's to answer (D2 replaces the hard-coded deny list): an
        // uncovered operation's slots pin exactly as v1's deny list pinned them.
        let Some(op) = semantic_ops::registry(operation) else {
            for param in &func.params {
                self.mark(
                    name,
                    Site::Value(*param),
                    func.values[param.0 as usize].ty,
                    types,
                    None,
                    InfReason::UnsupportedOperation,
                );
            }
            self.mark(
                name,
                Site::Ret,
                func.sig.result,
                types,
                None,
                InfReason::UnsupportedOperation,
            );
            return;
        };
        let slots: Vec<(Site, TypeId)> = func
            .params
            .iter()
            .map(|param| (Site::Value(*param), func.values[param.0 as usize].ty))
            .collect();
        let ret = Some((Site::Ret, func.sig.result));
        self.standard_slots(name, op, &slots, ret, types);
        self.standard_flows(name, op.id, &slots, ret, types);
    }

    /// Summarizes a byte-string-widened `String` method body (`BstrStringOp`) strictly under
    /// `crate::lower::bstr::enabled()` via its slot requirements.
    fn summarize_slot_reqs(
        &mut self,
        name: &JsName,
        func: &Func,
        types: &TypeTable,
        slot_reqs: &[SlotRequirement],
    ) {
        for param in &func.params {
            self.ensure(
                name,
                Site::Value(*param),
                func.values[param.0 as usize].ty,
                types,
            );
        }
        self.ensure(name, Site::Ret, func.sig.result, types);
        let slots: Vec<(Site, TypeId)> = func
            .params
            .iter()
            .map(|param| (Site::Value(*param), func.values[param.0 as usize].ty))
            .collect();
        let ret = Some((Site::Ret, func.sig.result));
        self.apply_slot_reqs(name, slot_reqs, &slots, ret, types);
    }

    fn apply_slot_reqs(
        &mut self,
        name: &JsName,
        slot_reqs: &[SlotRequirement],
        slots: &[(Site, TypeId)],
        ret: Option<(Site, TypeId)>,
        types: &TypeTable,
    ) {
        let at = |slot: SignatureSlot| match slot {
            SignatureSlot::Result => ret,
            SignatureSlot::Argument(index) => slots.get(usize::from(index)).copied(),
        };
        for requirement in slot_reqs {
            let Some((site, ty)) = at(requirement.slot) else {
                continue;
            };
            if requirement.reference == ReferenceRequirement::Mutable {
                self.mark_mut(name, site, ty, types, None);
            }
            let prefix: &[Step] = if requirement.reference == ReferenceRequirement::Value {
                &[]
            } else {
                &[Step::Deref]
            };
            self.constrain(name, site, ty, types, prefix, requirement.carriers);
        }
    }

    /// The mutability marks and carrier-set constraints of one standard operation's slot
    /// requirements, applied to a summarized body's own slots or (the D2 cut) a call site's
    /// arguments. `Effects::mutates` and the requirement's reference kind replace v1's
    /// `mutable_receiver` table; the requirement's carriers constrain the node the family
    /// sub-solves label.
    fn standard_slots(
        &mut self,
        name: &JsName,
        op: &'static SemanticOp,
        slots: &[(Site, TypeId)],
        ret: Option<(Site, TypeId)>,
        types: &TypeTable,
    ) {
        self.apply_slot_reqs(name, op.slots, slots, ret, types);
        if matches!(
            op.id,
            StandardOperation::F64SpecToString | StandardOperation::StringWithCapacity
        ) {
            if let Some((site, ty)) = ret {
                self.mark_mut(name, site, ty, types, None);
            }
        }
    }

    /// Intersects the carrier set a native label of the node at exactly `(site, prefix)` may
    /// take.
    fn constrain(
        &mut self,
        item: &JsName,
        site: Site,
        ty: TypeId,
        types: &TypeTable,
        prefix: &[Step],
        carriers: &'static [Carrier],
    ) {
        for info in self.walk(ty, types) {
            if info.path != prefix {
                continue;
            }
            let key = NodeKey::Value {
                item: item.clone(),
                site,
                path: info.path.clone(),
            };
            self.ensure_key(&key, &info);
            let facts = self
                .graph
                .nodes
                .get_mut(&key)
                .expect("the node was just ensured");
            let set: BTreeSet<Carrier> = carriers.iter().copied().collect();
            facts.carriers = Some(match facts.carriers.take() {
                None => set,
                Some(existing) => existing.intersection(&set).copied().collect(),
            });
        }
    }

    /// The slot flows of v1's `summarize_standard_operation`, over a summarized body's own
    /// slots or a cut call site's arguments, as registry-described flows
    /// ([`Self::flow_suffixes`]: matches union, one-sided candidates stay free).
    fn standard_flows(
        &mut self,
        name: &JsName,
        operation: StandardOperation,
        slots: &[(Site, TypeId)],
        ret: Option<(Site, TypeId)>,
        types: &TypeTable,
    ) {
        let param_ty = |index: usize| slots.get(index).copied();
        let flow = |this: &mut Self,
                    a: Option<(Site, TypeId)>,
                    a_prefix: &[Step],
                    b: Option<(Site, TypeId)>,
                    b_prefix: &[Step]| {
            if let (Some((a, a_ty)), Some((b, b_ty))) = (a, b) {
                this.flow_suffixes(name, a, a_ty, a_prefix, b, b_ty, b_prefix, types);
            }
        };
        match operation {
            StandardOperation::VecPush => flow(
                self,
                param_ty(1),
                &[],
                param_ty(0),
                &[Step::Deref, Step::Element],
            ),
            StandardOperation::VecIndex | StandardOperation::VecIndexMut => flow(
                self,
                param_ty(0),
                &[Step::Deref, Step::Element],
                ret,
                &[Step::Deref],
            ),
            StandardOperation::VecAsSlice | StandardOperation::VecAsMutSlice => flow(
                self,
                param_ty(0),
                &[Step::Deref, Step::Element],
                ret,
                &[Step::Deref, Step::Element],
            ),
            StandardOperation::VecRemove => {
                flow(self, param_ty(0), &[Step::Deref, Step::Element], ret, &[])
            }
            StandardOperation::VecExtend => {
                flow(self, param_ty(1), &[], param_ty(0), &[Step::Deref])
            }
            _ => {}
        }
    }

    /// Drop glue of a registry-owned candidate type: the body is summarized as a mutable pointer
    /// parameter and nothing else. Canonical `Box` reports its standard-library destructor as an
    /// own drop, but that destructor is exactly the allocator machinery the Box registry entry
    /// replaces; custom pointee drop is checked independently by the proof gate.
    fn summarize_drop_glue(
        &mut self,
        name: &JsName,
        func: &Func,
        types: &TypeTable,
        dropped: TypeId,
    ) -> bool {
        let candidate = types.get(dropped).is_some_and(|descriptor| {
            use crate::jir::types::RepresentationCandidate;
            match descriptor.meta.candidate {
                RepresentationCandidate::None => false,
                RepresentationCandidate::Box => true,
                RepresentationCandidate::Vec | RepresentationCandidate::String => {
                    !descriptor.layout.has_own_drop
                }
            }
        });
        if !candidate {
            // Structural and user glue are ordinary safe bodies: collected, not summarized.
            return false;
        }
        if let Some(param) = func.params.first() {
            let ty = func.values[param.0 as usize].ty;
            self.ensure(name, Site::Value(*param), ty, types);
            self.mark_mut(name, Site::Value(*param), ty, types, None);
        }
        true
    }

    // -- program-level boundaries -------------------------------------------------------------

    /// Vtable dispatch groups from decoded vtable bodies. The target set of a vtable slot is
    /// complete (every reachable vtable is an item of the table), so the group is not pinned;
    /// the receiver is, because its concrete storage is dispatched with carriers no call edge
    /// models.
    fn vtable_groups(&mut self) {
        for (name, item) in self.table {
            if !self.live.contains(name) {
                continue;
            }
            let Some(body) = item.jir.body.as_ref() else {
                continue;
            };
            let BodyKind::Vtable {
                trait_ref, entries, ..
            } = &body.kind
            else {
                continue;
            };
            for (slot, entry) in entries.iter().enumerate() {
                match entry {
                    VtableEntry::Method(target) => {
                        self.vtable_targets.insert(target.item);
                        let key = GroupKey::Vtable {
                            trait_ref: *trait_ref,
                            slot: slot as u32,
                        };
                        if !self.member_edges(target.item, key) {
                            self.pinned_groups.insert(key);
                        }
                    }
                    VtableEntry::Drop(Some(target)) => {
                        self.vtable_targets.insert(target.item);
                        self.pin_receiver(target.item);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Fn-pointer dispatch groups from reify and closure coercions. Reifications inside
    /// unlowered items cannot be enumerated, so every fn-pointer group additionally pins its own
    /// slots; the structure still collapses all known targets of one signature for the report
    /// and for a later, provenance-precise refinement.
    fn fn_pointer_groups(&mut self, scan: &RefScan) {
        for (fnptr, targets) in &scan.fn_pointer_targets {
            let key = GroupKey::FnPtr(*fnptr);
            self.pinned_groups.insert(key);
            for target in targets {
                self.member_edges(*target, key);
            }
        }
        let pinned: Vec<GroupKey> = self.pinned_groups.iter().copied().collect();
        let group_nodes: Vec<NodeKey> = self
            .graph
            .nodes
            .keys()
            .filter(|key| matches!(key, NodeKey::Group { key, .. } if pinned.contains(key)))
            .cloned()
            .collect();
        for node in group_nodes {
            self.graph
                .nodes
                .get_mut(&node)
                .expect("the node was just listed")
                .reasons
                .insert(InfReason::UnknownCall);
        }
    }

    /// Must-equal edges between a dispatch target's own slots and the group's. The receiver
    /// (parameter 0 of a vtable method) is pinned instead: its pointee is typed per target.
    /// Returns whether the target had a lowered body to edge against.
    fn member_edges(&mut self, target: ItemId, key: GroupKey) -> bool {
        let Some(target_name) = self.lowered.get(&target).cloned() else {
            return false;
        };
        let Some(Ok(func)) = self.funcs.get(&target_name) else {
            return false;
        };
        let Some(types) = self.types_of(&target_name) else {
            return false;
        };
        self.walk_cache.clear();
        let vtable = matches!(key, GroupKey::Vtable { .. });
        let params: Vec<(u32, ValueId, TypeId)> = func
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| (index as u32, *param, func.values[param.0 as usize].ty))
            .collect();
        let result_ty = func.sig.result;
        for (index, param, ty) in params {
            if vtable && index == 0 {
                self.mark(
                    &target_name,
                    Site::Value(param),
                    ty,
                    types,
                    None,
                    InfReason::UnknownCall,
                );
                continue;
            }
            self.slot_edges(
                &target_name,
                Site::Value(param),
                ty,
                types,
                key,
                BoundarySlot::Param(index),
            );
        }
        self.slot_edges(
            &target_name,
            Site::Ret,
            result_ty,
            types,
            key,
            BoundarySlot::Ret,
        );
        self.walk_cache.clear();
        true
    }

    fn slot_edges(
        &mut self,
        item: &JsName,
        site: Site,
        ty: TypeId,
        types: &TypeTable,
        key: GroupKey,
        slot: BoundarySlot,
    ) {
        self.ensure(item, site, ty, types);
        for info in self.walk(ty, types) {
            let far = NodeKey::Group {
                key,
                slot,
                path: info.path.clone(),
            };
            self.ensure_key(&far, &info);
            self.graph.edges.insert(Edge {
                a: NodeKey::Value {
                    item: item.clone(),
                    site,
                    path: info.path.clone(),
                },
                b: far,
                kind: EdgeKind::Shared,
            });
        }
    }

    /// Pins the receiver's pointee of a dynamically dispatched drop or method.
    fn pin_receiver(&mut self, target: ItemId) {
        let Some(name) = self.lowered.get(&target).cloned() else {
            return;
        };
        let Some(Ok(func)) = self.funcs.get(&name) else {
            return;
        };
        let Some(types) = self.types_of(&name) else {
            return;
        };
        self.walk_cache.clear();
        if let Some(param) = func.params.first().copied() {
            let ty = func.values[param.0 as usize].ty;
            self.mark(
                &name,
                Site::Value(param),
                ty,
                types,
                None,
                InfReason::UnknownCall,
            );
        }
        self.walk_cache.clear();
    }

    /// Export round trips (v1 `connect_export_round_trips`): the slots of one exported aggregate
    /// component answer as one, because JavaScript may return one export's value into another.
    ///
    /// The slots are also recorded ([`ValueGraph::exported_slots`]) for the lane sidecar. Until
    /// export wrappers adapt carriers in both directions, every exported slot is pinned to the
    /// baseline JavaScript ABI. Values that never reach an exported boundary are not recorded.
    fn exports(&mut self) {
        let mut components: BTreeMap<(TypeId, Vec<Step>), Vec<NodeKey>> = BTreeMap::new();
        for (name, func) in self.funcs {
            let Ok(func) = func else { continue };
            let exported = self
                .table
                .get(name)
                .is_some_and(|item| item.linkage.exported);
            if !exported {
                continue;
            }
            let Some(types) = self.types_of(name) else {
                continue;
            };
            self.walk_cache.clear();
            let mut slots: Vec<(Site, TypeId)> = func
                .params
                .iter()
                .map(|param| (Site::Value(*param), func.values[param.0 as usize].ty))
                .collect();
            slots.push((Site::Ret, func.sig.result));
            for (site, ty) in slots {
                let root = exported_root(types, ty);
                self.mark(name, site, ty, types, None, InfReason::Exported);
                for info in self.walk(ty, types) {
                    self.graph.exported_slots.insert(NodeKey::Value {
                        item: name.clone(),
                        site,
                        path: info.path.clone(),
                    });
                }
                for info in self.walk(ty, types) {
                    let stripped: Vec<Step> = info
                        .path
                        .iter()
                        .copied()
                        .skip_while(|step| *step == Step::Deref)
                        .collect();
                    components
                        .entry((root, stripped))
                        .or_default()
                        .push(NodeKey::Value {
                            item: name.clone(),
                            site,
                            path: info.path.clone(),
                        });
                }
            }
        }
        for members in components.values() {
            let Some((first, rest)) = members.split_first() else {
                continue;
            };
            for member in rest {
                if member != first {
                    self.graph.edges.insert(Edge {
                        a: first.clone(),
                        b: member.clone(),
                        kind: EdgeKind::Export,
                    });
                }
            }
        }
    }

    /// Pins the parameters and return of every item something unmodeled can call: an item named
    /// outside a plain direct call, an item referenced by unlowered baseline text, and an item
    /// with a kept name that is not part of the module's export clause. The pin is on the item's
    /// BOUNDARY nodes only; values that never flow to its boundary stay free.
    fn pins(&mut self, scan: &RefScan) {
        let mut pinned: BTreeSet<ItemId> = scan.named.clone();
        // The refs of unlowered items, except the vtables and statics whose references are
        // modeled from their decoded bodies.
        let by_name: BTreeMap<&str, ItemId> = self
            .lowered
            .iter()
            .map(|(item, name)| (name.as_str(), *item))
            .collect();
        for (name, item) in self.table {
            if !self.live.contains(name) || self.funcs.get(name).is_some_and(Result::is_ok) {
                continue;
            }
            match item.jir.body.as_ref().map(|body| &body.kind) {
                Some(BodyKind::Vtable { .. }) => continue,
                Some(BodyKind::Static { init, .. }) => {
                    // A function a static's initializer names has escaped as a value.
                    let mut named = BTreeSet::new();
                    refs::scan_tree(init, &mut named);
                    pinned.extend(named);
                    continue;
                }
                _ => {}
            }
            for referenced in &item.refs {
                if let Some(id) = by_name.get(referenced.as_str()) {
                    pinned.insert(*id);
                }
            }
        }
        // Vtable methods and drop glue are modeled as groups and drop edges.
        for target in &self.vtable_targets.clone() {
            pinned.remove(target);
        }
        for name in self.funcs.keys() {
            let internal = self.table.get(name).is_none_or(|item| {
                item.linkage == crate::item::Linkage::internal() || item.linkage.exported
            });
            if !internal {
                if let Some(Ok(func)) = self.funcs.get(name) {
                    pinned.insert(func.item);
                }
            }
        }
        for item in pinned {
            let Some(name) = self.lowered.get(&item).cloned() else {
                continue;
            };
            let Some(Ok(func)) = self.funcs.get(&name) else {
                continue;
            };
            let Some(types) = self.types_of(&name) else {
                continue;
            };
            self.walk_cache.clear();
            let slots: Vec<(Site, TypeId)> = func
                .params
                .iter()
                .map(|param| (Site::Value(*param), func.values[param.0 as usize].ty))
                .chain(std::iter::once((Site::Ret, func.sig.result)))
                .collect();
            for (site, ty) in slots {
                self.mark(&name, site, ty, types, None, InfReason::UnknownCall);
                for info in self.walk(ty, types) {
                    self.graph.pinned_boundaries.insert(NodeKey::Value {
                        item: name.clone(),
                        site,
                        path: info.path,
                    });
                }
            }
        }
        self.walk_cache.clear();
    }

    /// The artifact-wide allocator answer, exactly v1's `apply_allocator_provider`: unless some
    /// crate proves the default global allocator, every discovered value keeps a unary reason.
    fn allocator(&mut self) {
        let provider = self
            .table
            .values()
            .fold(AllocatorProvider::Unknown, |provider, item| {
                provider.merge(item.late_ir.artifact_allocator_provider())
            });
        let reason = match provider {
            AllocatorProvider::Default => return,
            AllocatorProvider::Custom => InfReason::CustomAllocator,
            AllocatorProvider::Unknown | AllocatorProvider::Conflict => InfReason::UnknownAllocator,
        };
        for (key, facts) in &mut self.graph.nodes {
            if *key != NodeKey::Baseline {
                facts.reasons.insert(reason);
            }
        }
    }
}

/// Per-function context: types, use counts, and definition sites.
struct FuncCx<'f> {
    name: &'f JsName,
    func: &'f Func,
    uses: Vec<u32>,
    defs: Vec<Option<&'f OpKind>>,
    def_block: Vec<Option<u32>>,
}

impl FuncCx<'_> {
    fn ty(&self, v: ValueId) -> TypeId {
        self.func.values[v.0 as usize].ty
    }

    fn cell_ty(&self, c: CellId) -> TypeId {
        self.func.cells[c.0 as usize].ty
    }

    fn root(&self, root: Root) -> (Site, TypeId) {
        match root {
            Root::Value(v) => (Site::Value(v), self.ty(v)),
            Root::Cell(c) => (Site::Cell(c), self.cell_ty(c)),
        }
    }

    /// Whether `v` is dead and unaliased after its one use in `block`: the baseline-to-native
    /// direction's legality at a by-value edge.
    ///
    /// The proof is deliberately small: `v` is used exactly once in the whole function, it was
    /// defined in the same block as the use (so a loop cannot cross the edge twice with one
    /// textual use), and its definition chain bottoms out in a fresh value rather than a read out
    /// of storage something else still holds.
    // ponytail: same-block plus single-use is the floor; real liveness would legalize more
    // sources and belongs to W4b if the commit rate needs it.
    fn dead_unaliased(&self, v: ValueId, block: u32) -> bool {
        self.uses[v.0 as usize] == 1 && self.def_block[v.0 as usize] == Some(block) && self.fresh(v)
    }

    /// Whether the value's definition made fresh storage nothing else names.
    fn fresh(&self, v: ValueId) -> bool {
        match self.defs[v.0 as usize] {
            Some(OpKind::Move(inner)) => self.fresh(*inner),
            Some(
                OpKind::Const(_)
                | OpKind::Copy(_)
                | OpKind::Aggregate { .. }
                | OpKind::Repeat { .. }
                | OpKind::Call { .. }
                | OpKind::Intrinsic { .. }
                | OpKind::Materialize(_)
                | OpKind::Convert { .. },
            ) => true,
            _ => false,
        }
    }
}

/// The path prefix of field `index` of an aggregate at `ty`, or `None` when the type is not
/// walked and the flowing value must be pinned in place.
fn aggregate_prefix(types: &TypeTable, ty: TypeId, variant: u32, index: u32) -> Option<Vec<Step>> {
    let descriptor = types.get(types.peeled(ty))?;
    match &descriptor.kind {
        TypeKind::Tuple(_) | TypeKind::Closure { .. } => Some(vec![Step::Field(index)]),
        TypeKind::Adt { form, variants, .. } => match form {
            NominalForm::Struct if variants.len() == 1 => Some(vec![Step::Field(index)]),
            NominalForm::Enum => Some(vec![Step::Variant(variant), Step::Field(index)]),
            _ => None,
        },
        _ => None,
    }
}

/// The pointee of a reference or raw pointer, or the type itself.
fn deref_pointee(types: &TypeTable, ty: TypeId) -> TypeId {
    match types
        .get(types.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            *pointee
        }
        _ => ty,
    }
}

/// Whether a coercion target is (a pointer to) a `dyn` object.
fn coerces_to_dynamic(types: &TypeTable, to: TypeId) -> bool {
    let target = types.peeled(deref_pointee(types, types.peeled(to)));
    matches!(
        types.get(target).map(|descriptor| &descriptor.kind),
        Some(TypeKind::Dynamic { .. })
    )
}

/// The root type of an exported slot: references peeled, exactly v1's `exported_root_type`.
fn exported_root(types: &TypeTable, mut ty: TypeId) -> TypeId {
    loop {
        match types
            .get(types.peeled(ty))
            .map(|descriptor| &descriptor.kind)
        {
            Some(TypeKind::Reference { pointee, .. }) => ty = *pointee,
            _ => return ty,
        }
    }
}

/// Every static a constant tree references, at any depth.
fn collect_static_refs(tree: &ConstTree, out: &mut Vec<DefinitionId>) {
    match tree {
        ConstTree::Global {
            target: GlobalReference::Static { definition, .. },
            ..
        } => out.push(*definition),
        ConstTree::Aggregate(fields)
        | ConstTree::Slice(fields)
        | ConstTree::Variant { fields, .. } => {
            for field in fields {
                collect_static_refs(field, out);
            }
        }
        ConstTree::Reference(inner) | ConstTree::Dyn { data: inner, .. } => {
            collect_static_refs(inner, out)
        }
        _ => {}
    }
}

/// The JIR-R place path as value-path steps, or `None` when a projection falls outside the value
/// model (v1 `place_path`).
fn proj_steps(path: &[Proj]) -> Option<Vec<Step>> {
    path.iter()
        .map(|proj| match proj {
            Proj::Deref => Some(Step::Deref),
            Proj::Field { field, .. } => Some(Step::Field(*field)),
            Proj::Downcast { variant } => Some(Step::Variant(*variant)),
            Proj::Index(_) | Proj::ConstantIndex { .. } | Proj::Subslice { .. } => None,
        })
        .collect()
}

/// Discovery: the candidate positions of one type (v1 `discover_values` onto the v2 type table).
fn walk_type(
    ty: TypeId,
    types: &TypeTable,
    path: &mut Vec<Step>,
    inherited: &BTreeSet<InfReason>,
    inherited_pin: bool,
    out: &mut Vec<PathInfo>,
    stack: &mut BTreeSet<TypeId>,
) {
    let Some(descriptor) = types.get(ty) else {
        return;
    };
    let (own, pin) = type_reasons(descriptor);
    let mut reasons = inherited.clone();
    reasons.extend(own);
    let pin = inherited_pin || pin;

    let kind = match descriptor.meta.candidate {
        crate::jir::types::RepresentationCandidate::Box => Some(CandidateKind::Box),
        crate::jir::types::RepresentationCandidate::Vec => Some(CandidateKind::Vec),
        crate::jir::types::RepresentationCandidate::String => Some(CandidateKind::String),
        crate::jir::types::RepresentationCandidate::None => None,
    };
    if kind == Some(CandidateKind::Vec) {
        let element = match &descriptor.kind {
            TypeKind::Adt { args, .. } => args.first().copied(),
            _ => None,
        };
        match element.and_then(|element| types.get(element)) {
            Some(element) if !element.layout.sized => {
                reasons.insert(InfReason::Unsized);
            }
            Some(element) if element.layout.zst => {
                reasons.insert(InfReason::ZeroSized);
            }
            Some(element) if element.layout.size.is_none() || element.layout.align.is_none() => {
                reasons.insert(InfReason::UnknownLayout);
            }
            Some(_) => {}
            None => {
                reasons.insert(InfReason::UnknownLayout);
            }
        }
    }
    if kind == Some(CandidateKind::Box) {
        let pointee = match &descriptor.kind {
            TypeKind::Adt { args, .. } => args.first().copied(),
            _ => None,
        };
        match pointee.and_then(|pointee| types.get(pointee)) {
            Some(pointee) if pointee.layout.has_own_drop => {
                reasons.insert(InfReason::UserDefinedDrop);
            }
            Some(pointee) if !pointee.layout.sized => {
                reasons.insert(InfReason::Unsized);
            }
            Some(pointee) if pointee.layout.zst => {
                reasons.insert(InfReason::ZeroSized);
            }
            Some(pointee) if pointee.layout.size.is_none() || pointee.layout.align.is_none() => {
                reasons.insert(InfReason::UnknownLayout);
            }
            Some(_) => {}
            None => {
                reasons.insert(InfReason::UnknownLayout);
            }
        }
    }
    if let Some(kind) = kind {
        out.push(PathInfo {
            path: path.clone(),
            kind,
            ty,
            reasons: reasons.clone(),
            host_pin: pin,
        });
        // A native Vec owns ordinary Rust elements whose nested heap values need their own
        // decisions; String and Box internals are representation boundaries.
        if kind != CandidateKind::Vec {
            return;
        }
    }

    if !stack.insert(ty) {
        return;
    }
    match &descriptor.kind {
        TypeKind::Tuple(fields) => {
            for (index, field) in fields.iter().enumerate() {
                path.push(Step::Field(index as u32));
                walk_type(*field, types, path, &reasons, pin, out, stack);
                path.pop();
            }
        }
        TypeKind::Closure { upvars, .. } => {
            for (index, upvar) in upvars.iter().enumerate() {
                path.push(Step::Field(index as u32));
                walk_type(*upvar, types, path, &reasons, pin, out, stack);
                path.pop();
            }
        }
        TypeKind::Adt {
            args,
            form: NominalForm::Struct,
            ..
        } if kind == Some(CandidateKind::Vec) => {
            if let Some(element) = args.first() {
                path.push(Step::Element);
                walk_type(*element, types, path, &reasons, pin, out, stack);
                path.pop();
            }
        }
        TypeKind::Adt {
            form: NominalForm::Struct,
            variants,
            ..
        } if variants.len() == 1 => {
            for (index, field) in variants[0].fields.iter().enumerate() {
                path.push(Step::Field(index as u32));
                walk_type(field.ty, types, path, &reasons, pin, out, stack);
                path.pop();
            }
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } => {
            for (variant_index, variant) in variants.iter().enumerate() {
                for (index, field) in variant.fields.iter().enumerate() {
                    path.push(Step::Variant(variant_index as u32));
                    path.push(Step::Field(index as u32));
                    walk_type(field.ty, types, path, &reasons, pin, out, stack);
                    path.pop();
                    path.pop();
                }
            }
        }
        TypeKind::Reference { pointee, .. } => {
            path.push(Step::Deref);
            walk_type(*pointee, types, path, &reasons, pin, out, stack);
            path.pop();
        }
        TypeKind::Slice(element) => {
            path.push(Step::Element);
            walk_type(*element, types, path, &reasons, pin, out, stack);
            path.pop();
        }
        TypeKind::Pattern(inner) => {
            walk_type(*inner, types, path, &reasons, pin, out, stack);
        }
        _ => {}
    }
    stack.remove(&ty);
}

/// The unary reasons a type's own facts imply (v1 `blockers_for_type`), and whether the type
/// carries a pinned host boundary (an edge, not a reason).
///
/// `Unsized` and `ZeroSized` describe the value at the node itself and are not inherited by what
/// a walk finds below it: the element of a slice is sized however unsized the slice is.
fn type_reasons(descriptor: &TypeDescriptor) -> (BTreeSet<InfReason>, bool) {
    use crate::jir::types::{
        AllocatorIdentity, NominalAbi, PinnedBoundary, RepresentationCandidate,
    };
    let mut reasons = BTreeSet::new();
    let meta = descriptor.meta;
    if descriptor.layout.has_own_drop
        && !matches!(
            meta.candidate,
            RepresentationCandidate::Box
                | RepresentationCandidate::Vec
                | RepresentationCandidate::String
        )
    {
        reasons.insert(InfReason::UserDefinedDrop);
    }
    match meta.abi {
        NominalAbi::C => {
            reasons.insert(InfReason::CAbi);
        }
        NominalAbi::Transparent => {
            reasons.insert(InfReason::UnsupportedOperation);
        }
        NominalAbi::Rust => {}
    }
    let mut pin = false;
    match meta.boundary {
        PinnedBoundary::Host => pin = true,
        PinnedBoundary::ViewAbi => {
            reasons.insert(InfReason::ViewAbi);
        }
        PinnedBoundary::None => {}
    }
    match meta.allocator {
        AllocatorIdentity::Custom => {
            reasons.insert(InfReason::CustomAllocator);
        }
        AllocatorIdentity::Unknown => {
            reasons.insert(InfReason::UnknownAllocator);
        }
        AllocatorIdentity::NotApplicable | AllocatorIdentity::Global => {}
    }
    let candidate = meta.candidate != RepresentationCandidate::None;
    if candidate {
        if !descriptor.layout.sized {
            reasons.insert(InfReason::Unsized);
        }
        if descriptor.layout.zst {
            reasons.insert(InfReason::ZeroSized);
        }
    }
    (reasons, pin)
}

/// Shared fixtures for the valueflow, legality, and problem-construction tests.
#[cfg(test)]
pub(crate) mod harness {
    use std::collections::BTreeMap;

    use super::*;
    use crate::item::{ItemKind, ItemMap, Linkage};
    use crate::jir::interns::StrId;
    use crate::jir::origin::OriginId;
    use crate::jir::repr::FuncBuilder;
    use crate::jir::semantic::{AbiSignature, Body, DirectItemRef, ItemId, Signature};
    use crate::jir::types::{
        AllocatorIdentity, FieldDescriptor, FieldKey, JsFacts, LayoutFacts, NominalAbi,
        PinnedBoundary, RepresentationCandidate, TypeDescriptor, TypeMeta, VariantDescriptor,
    };
    use crate::late_ir::LateIrAnalysisSummary;

    pub const NO_ORIGIN: OriginId = OriginId(0);
    pub const UNIT: TypeId = TypeId(1);
    pub const U32: TypeId = TypeId(2);
    pub const STRING: TypeId = TypeId(3);
    pub const REF_STRING: TypeId = TypeId(4);
    pub const MUT_STRING: TypeId = TypeId(5);
    pub const VEC: TypeId = TypeId(6);
    pub const MUT_VEC: TypeId = TypeId(7);
    /// `(String, u32)`.
    pub const PAIR: TypeId = TypeId(8);
    /// `Option<String>`: variant 0 empty, variant 1 holds the string.
    pub const OPTION: TypeId = TypeId(9);
    pub const FNPTR: TypeId = TypeId(10);
    /// A `FnDef` resolving to `item(90)`.
    pub const FN_DEF: TypeId = TypeId(11);
    pub const SLICE: TypeId = TypeId(12);
    pub const REF_SLICE: TypeId = TypeId(13);
    pub const DYN: TypeId = TypeId(14);
    pub const REF_DYN: TypeId = TypeId(15);
    /// A non-candidate struct with its own destructor holding a `String`.
    pub const DROPPY: TypeId = TypeId(16);

    pub fn item(local_hash: u64) -> ItemId {
        ItemId {
            definition: def(local_hash),
            instantiation: u128::from(local_hash),
        }
    }

    pub fn def(local_hash: u64) -> DefinitionId {
        DefinitionId {
            crate_id: 0,
            local_hash,
        }
    }

    pub fn sized() -> LayoutFacts {
        LayoutFacts {
            size: Some(8),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        }
    }

    pub fn descriptor(
        id: TypeId,
        kind: TypeKind,
        meta: TypeMeta,
        layout: LayoutFacts,
    ) -> TypeDescriptor {
        TypeDescriptor {
            id,
            name: None,
            kind,
            layout,
            meta,
            js: JsFacts {
                int: None,
                is_indirect: true,
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

    fn variant(fields: Vec<TypeId>) -> VariantDescriptor {
        VariantDescriptor {
            name: StrId(0),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| FieldDescriptor {
                    key: FieldKey::Index(index as u32),
                    ty,
                    offset: 0,
                })
                .collect(),
            logical_discr: 0,
            physical_tag: None,
            result_ok: false,
            saved_local_keys: Vec::new(),
        }
    }

    fn candidate(kind: RepresentationCandidate) -> TypeMeta {
        TypeMeta {
            candidate: kind,
            abi: NominalAbi::Rust,
            boundary: PinnedBoundary::None,
            allocator: AllocatorIdentity::Global,
            iterator: crate::jir::types::IteratorIdentity::None,
        }
    }

    fn adt(def_hash: u64, args: Vec<TypeId>, variants: Vec<VariantDescriptor>) -> TypeKind {
        TypeKind::Adt {
            def: def(def_hash),
            args,
            form: NominalForm::Struct,
            variants,
            niche: None,
        }
    }

    pub fn base_interns() -> Interns {
        let mut interns = Interns::new();
        let meta = TypeMeta::default;
        for descriptor in [
            descriptor(UNIT, TypeKind::Tuple(Vec::new()), meta(), sized()),
            descriptor(
                U32,
                TypeKind::Integer {
                    signed: false,
                    bits: 32,
                },
                meta(),
                sized(),
            ),
            descriptor(
                STRING,
                adt(100, Vec::new(), vec![variant(Vec::new())]),
                candidate(RepresentationCandidate::String),
                sized(),
            ),
            descriptor(
                REF_STRING,
                TypeKind::Reference {
                    pointee: STRING,
                    mutable: false,
                },
                meta(),
                sized(),
            ),
            descriptor(
                MUT_STRING,
                TypeKind::Reference {
                    pointee: STRING,
                    mutable: true,
                },
                meta(),
                sized(),
            ),
            descriptor(
                VEC,
                adt(101, vec![STRING], vec![variant(Vec::new())]),
                candidate(RepresentationCandidate::Vec),
                sized(),
            ),
            descriptor(
                MUT_VEC,
                TypeKind::Reference {
                    pointee: VEC,
                    mutable: true,
                },
                meta(),
                sized(),
            ),
            descriptor(PAIR, TypeKind::Tuple(vec![STRING, U32]), meta(), sized()),
            descriptor(
                OPTION,
                TypeKind::Adt {
                    def: def(102),
                    args: vec![STRING],
                    form: NominalForm::Enum,
                    variants: vec![variant(Vec::new()), variant(vec![STRING])],
                    niche: None,
                },
                meta(),
                sized(),
            ),
            descriptor(
                FNPTR,
                TypeKind::FnPtr {
                    sig: crate::jir::interns::SigId(0),
                },
                meta(),
                sized(),
            ),
            descriptor(
                FN_DEF,
                TypeKind::FnDef {
                    def: def(90),
                    sig: crate::jir::interns::SigId(0),
                    instance: Some(DirectItemRef {
                        item: item(90),
                        link_name: StrId(0),
                    }),
                },
                meta(),
                sized(),
            ),
            descriptor(
                SLICE,
                TypeKind::Slice(STRING),
                meta(),
                LayoutFacts {
                    sized: false,
                    ..LayoutFacts::default()
                },
            ),
            descriptor(
                REF_SLICE,
                TypeKind::Reference {
                    pointee: SLICE,
                    mutable: false,
                },
                meta(),
                sized(),
            ),
            descriptor(
                DYN,
                TypeKind::Dynamic {
                    principal: Some(def(103)),
                    auto: Vec::new(),
                },
                meta(),
                LayoutFacts {
                    sized: false,
                    ..LayoutFacts::default()
                },
            ),
            descriptor(
                REF_DYN,
                TypeKind::Reference {
                    pointee: DYN,
                    mutable: false,
                },
                meta(),
                sized(),
            ),
            descriptor(
                DROPPY,
                adt(104, Vec::new(), vec![variant(vec![STRING])]),
                meta(),
                LayoutFacts {
                    needs_drop: true,
                    has_own_drop: true,
                    ..sized()
                },
            ),
        ] {
            assert!(interns.types.insert(descriptor));
        }
        interns
    }

    pub fn sig(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        let parameters = arguments.len() as u16;
        Signature {
            arguments,
            result,
            abi: AbiSignature {
                parameters,
                spread: None,
                caller_location: false,
                inline_never: false,
            },
        }
    }

    fn link_item(name: &str) -> LinkItem {
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: String::new(),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: LateIrAnalysisSummary::allocator_provider(AllocatorProvider::Default),
            baseline_shadow: Default::default(),
            jir: Default::default(),
        }
    }

    /// A test program: items, their lowered `Func`s, and one shared interns table.
    pub struct Prog {
        pub interns: Interns,
        pub table: BTreeMap<JsName, LinkItem>,
        pub funcs: BTreeMap<JsName, Result<Func, Decline>>,
    }

    impl Prog {
        pub fn new() -> Prog {
            Prog {
                interns: base_interns(),
                table: BTreeMap::new(),
                funcs: BTreeMap::new(),
            }
        }

        pub fn add(&mut self, name: &str, func: Func) {
            self.add_semantic(name, func, BodySemantic::Ordinary);
        }

        pub fn add_semantic(&mut self, name: &str, func: Func, semantic: BodySemantic) {
            let mut item = link_item(name);
            item.jir.body = Some(Body {
                item: func.item,
                kind: BodyKind::Function,
                semantic,
                signature: func.sig.clone(),
                locals: Vec::new(),
                blocks: Vec::new(),
            });
            self.table.insert(JsName::new(name.to_owned()), item);
            self.funcs.insert(JsName::new(name.to_owned()), Ok(func));
        }

        /// An item with no lowered `Func`: a static, a vtable, or baseline text with `refs`.
        pub fn add_unlowered(&mut self, name: &str, body: Option<Body>, refs: &[&str]) {
            let mut item = link_item(name);
            item.jir.body = body;
            item.refs = refs.iter().map(|r| JsName::new((*r).to_owned())).collect();
            self.table.insert(JsName::new(name.to_owned()), item);
        }

        pub fn exported(&mut self, name: &str) {
            let item = self.table.get_mut(&JsName::new(name.to_owned())).unwrap();
            item.linkage = Linkage::exported(name.to_owned());
        }

        /// Replaces one type's descriptor, for the meta- and layout-driven legality tests.
        pub fn set_type(&mut self, id: TypeId, edit: impl FnOnce(&mut TypeDescriptor)) {
            let mut descriptor = self.interns.types.get(id).unwrap().clone();
            edit(&mut descriptor);
            self.interns.types.replace(descriptor);
        }

        /// Leaves the artifact allocator unproven, for the allocator legality test.
        pub fn unknown_allocator(&mut self) {
            for item in self.table.values_mut() {
                item.late_ir = LateIrAnalysisSummary::default();
            }
        }

        pub fn collect(&self) -> ValueGraph {
            let interns: BTreeMap<JsName, &Interns> = self
                .table
                .keys()
                .map(|name| (name.clone(), &self.interns))
                .collect();
            let live: BTreeSet<JsName> = self.table.keys().cloned().collect();
            super::collect(&self.table, &self.funcs, &interns, &live)
        }
    }

    pub fn value_node(item_name: &str, site: Site, path: &[Step]) -> NodeKey {
        NodeKey::Value {
            item: JsName::new(item_name.to_owned()),
            site,
            path: path.to_vec(),
        }
    }

    /// `fn callee(x: <arg>) -> <result>`, returning its parameter when the types agree.
    pub fn identity_func(id: u64, arg: TypeId, result: TypeId) -> Func {
        let mut builder = FuncBuilder::new(item(id), sig(vec![arg], result));
        let p = builder.param(arg, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        if arg == result {
            builder.terminate(entry, RTerm::Return(Some(p)));
        } else {
            builder.terminate(entry, RTerm::Return(None));
        }
        builder.finish()
    }

    /// `fn caller() { callee(<fresh string>) }` through `build`.
    pub fn call_func(
        id: u64,
        arg_ty: TypeId,
        callee: Callee,
        semantic: CallSemantic,
        host: Option<HostPayload>,
    ) -> Func {
        let mut builder = FuncBuilder::new(item(id), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let arg = builder.op(entry, OpKind::Const(ConstTree::Unit), arg_ty, NO_ORIGIN);
        builder.effect(
            entry,
            OpKind::Call {
                callee,
                function: None,
                args: vec![arg],
                track_caller: None,
                semantic,
                host,
            },
            NO_ORIGIN,
        );
        builder.terminate(entry, RTerm::Return(None));
        builder.finish()
    }

    pub fn direct(id: u64) -> Callee {
        Callee::Direct {
            item: item(id),
            link_name: StrId(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::harness::*;
    use super::*;
    use crate::analysis::legality::InfReason;
    use crate::jir::repr::FuncBuilder;
    use crate::jir::semantic::VtableEntry;

    fn reasons_of(graph: &ValueGraph, key: &NodeKey) -> Vec<InfReason> {
        graph.nodes[key].reasons.iter().copied().collect()
    }

    #[test]
    fn copies_moves_and_block_arguments_union_into_one_node() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], STRING));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let moved = b.op(entry, OpKind::Move(p), STRING, NO_ORIGIN);
        let next = b.block();
        let q = b.block_param(next, STRING, NO_ORIGIN);
        b.terminate(entry, RTerm::Goto(next, vec![moved]));
        b.terminate(next, RTerm::Return(Some(q)));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let unions: Vec<(NodeKey, NodeKey)> = graph.unions.iter().cloned().collect();
        assert_eq!(unions.len(), 3, "{}", graph.dump());
        let node = |site| value_node("f", site, &[]);
        assert!(graph
            .unions
            .contains(&(node(Site::Value(p)), node(Site::Value(moved)))));
        assert!(graph
            .unions
            .contains(&(node(Site::Value(moved)), node(Site::Value(q)))));
        assert!(graph
            .unions
            .contains(&(node(Site::Value(q)), node(Site::Ret))));
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn aggregates_fields_and_enum_variants_pair_by_path_suffix() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let n = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let pair = b.op(
            entry,
            OpKind::Aggregate {
                ty: PAIR,
                variant: 0,
                active_field: None,
                fields: vec![p, n],
            },
            PAIR,
            NO_ORIGIN,
        );
        let back = b.op(
            entry,
            OpKind::Field {
                base: pair,
                variant: None,
                field: 0,
            },
            STRING,
            NO_ORIGIN,
        );
        let opt = b.op(
            entry,
            OpKind::Aggregate {
                ty: OPTION,
                variant: 1,
                active_field: None,
                fields: vec![back],
            },
            OPTION,
            NO_ORIGIN,
        );
        let _ = opt;
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(graph.unions.contains(&(
            value_node("f", Site::Value(p), &[]),
            value_node("f", Site::Value(pair), &[Step::Field(0)]),
        )));
        assert!(graph.unions.contains(&(
            value_node("f", Site::Value(pair), &[Step::Field(0)]),
            value_node("f", Site::Value(back), &[]),
        )));
        // The enum payload is a walked path, not an unsupported pin.
        assert!(graph.unions.contains(&(
            value_node("f", Site::Value(back), &[]),
            value_node("f", Site::Value(opt), &[Step::Variant(1), Step::Field(0)]),
        )));
        for (key, facts) in &graph.nodes {
            assert!(facts.reasons.is_empty(), "{key}: {:?}", facts.reasons);
        }
    }

    #[test]
    fn borrows_union_storage_and_record_mutation() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let r = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: Vec::new(),
                mutable: true,
            },
            MUT_STRING,
            NO_ORIGIN,
        );
        let d = b.op(entry, OpKind::Deref(r), STRING, NO_ORIGIN);
        let _ = d;
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(graph.unions.contains(&(
            value_node("f", Site::Value(p), &[]),
            value_node("f", Site::Value(r), &[Step::Deref]),
        )));
        assert!(graph.unions.contains(&(
            value_node("f", Site::Value(r), &[Step::Deref]),
            value_node("f", Site::Value(d), &[]),
        )));
        assert!(graph.nodes[&value_node("f", Site::Value(p), &[])].mutable);
    }

    #[test]
    fn a_call_boundary_is_an_edge_with_the_dead_source_direction() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        prog.add(
            "caller",
            call_func(1, STRING, direct(2), CallSemantic::Ordinary, None),
        );
        let graph = prog.collect();
        let callee_param = {
            let Ok(func) = &prog.funcs[&JsName::new("callee".to_owned())] else {
                unreachable!()
            };
            func.params[0]
        };
        let edge = graph
            .edges
            .iter()
            .find(|edge| matches!(edge.kind, EdgeKind::Value { .. }))
            .expect("a by-value call edge");
        assert_eq!(edge.kind, EdgeKind::Value { source_dead: true });
        assert_eq!(edge.b, value_node("callee", Site::Value(callee_param), &[]));
        // No union crosses the boundary.
        for (a, b) in &graph.unions {
            let of = |key: &NodeKey| match key {
                NodeKey::Value { item, .. } => item.as_str().to_owned(),
                _ => String::new(),
            };
            assert_eq!(of(a), of(b));
        }
    }

    #[test]
    fn a_mutable_reference_argument_shares_storage_across_the_call() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, MUT_STRING, UNIT));
        prog.add(
            "caller",
            call_func(1, MUT_STRING, direct(2), CallSemantic::Ordinary, None),
        );
        let graph = prog.collect();
        let edge = graph
            .edges
            .iter()
            .find(|edge| edge.kind == EdgeKind::Shared)
            .expect("a shared storage edge");
        assert!(matches!(&edge.a, NodeKey::Value { path, .. } if path == &[Step::Deref]));
    }

    #[test]
    fn an_aliased_or_reused_source_is_not_dead_at_the_edge() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        // The argument is the caller's own parameter: not fresh, so not proven dead.
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("caller", b.finish());
        let graph = prog.collect();
        let edge = graph
            .edges
            .iter()
            .find(|edge| matches!(edge.kind, EdgeKind::Value { .. }))
            .expect("a by-value call edge");
        assert_eq!(edge.kind, EdgeKind::Value { source_dead: false });
    }

    #[test]
    fn a_standard_body_is_summarized_not_collected() {
        let mut prog = Prog::new();
        // `Vec::push(&mut self, value)`, with raw internals that must never reach the graph.
        let mut b = FuncBuilder::new(item(2), sig(vec![MUT_VEC, STRING], UNIT));
        let receiver = b.param(MUT_VEC, NO_ORIGIN);
        let value = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let raw = b.op(
            entry,
            OpKind::RawView {
                root: Root::Value(receiver),
                path: Vec::new(),
                mutable: true,
            },
            MUT_VEC,
            NO_ORIGIN,
        );
        let _ = (value, raw);
        b.terminate(entry, RTerm::Return(None));
        prog.add_semantic(
            "push",
            b.finish(),
            BodySemantic::Standard(StandardOperation::VecPush),
        );
        let graph = prog.collect();
        for (key, facts) in &graph.nodes {
            assert!(
                !facts.reasons.contains(&InfReason::RawProvenance),
                "{key} was collected, not summarized"
            );
        }
        // The receiver is mutable and the pushed value flows into its elements.
        assert!(graph.nodes[&value_node("push", Site::Value(receiver), &[Step::Deref])].mutable);
        assert!(graph.unions.contains(&(
            value_node("push", Site::Value(receiver), &[Step::Deref, Step::Element]),
            value_node("push", Site::Value(value), &[]),
        )));
    }

    #[test]
    fn an_unsupported_standard_operation_pins_its_slots() {
        let mut prog = Prog::new();
        prog.add_semantic(
            "pop",
            identity_func(2, MUT_VEC, UNIT),
            BodySemantic::Standard(StandardOperation::VecPop),
        );
        let graph = prog.collect();
        let Ok(func) = &prog.funcs[&JsName::new("pop".to_owned())] else {
            unreachable!()
        };
        let key = value_node("pop", Site::Value(func.params[0]), &[Step::Deref]);
        assert!(reasons_of(&graph, &key).contains(&InfReason::UnsupportedOperation));
    }

    #[test]
    fn candidate_drop_glue_is_summarized_and_a_drop_is_an_edge() {
        let mut prog = Prog::new();
        prog.add_semantic(
            "glue",
            identity_func(3, MUT_STRING, UNIT),
            BodySemantic::DropGlue(STRING),
        );
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.effect(
            entry,
            OpKind::Drop {
                root: Root::Value(p),
                path: Vec::new(),
                glue: DropGlue::Direct {
                    item: item(3),
                    link_name: crate::jir::interns::StrId(0),
                },
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let Ok(glue) = &prog.funcs[&JsName::new("glue".to_owned())] else {
            unreachable!()
        };
        let glue_param = value_node("glue", Site::Value(glue.params[0]), &[Step::Deref]);
        assert!(graph.nodes[&glue_param].mutable);
        // The adapter kind with a proven-dead source, never must-equal: a drop is the value's
        // last use, so one pinned dropped value must not tie every value sharing the glue.
        assert!(graph.edges.contains(&Edge {
            a: value_node("f", Site::Value(p), &[]),
            b: glue_param,
            kind: EdgeKind::Value { source_dead: true },
        }));
        assert!(graph.direct_calls[&JsName::new("f".to_owned())]
            .contains(&JsName::new("glue".to_owned())));
    }

    #[test]
    fn a_vtable_slot_is_a_group_and_the_receiver_is_pinned() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(2), sig(vec![MUT_STRING, STRING], UNIT));
        let receiver = b.param(MUT_STRING, NO_ORIGIN);
        let arg = b.param(STRING, NO_ORIGIN);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("method", b.finish());
        prog.add_unlowered(
            "vt",
            Some(crate::jir::semantic::Body {
                item: item(50),
                kind: BodyKind::Vtable {
                    concrete: STRING,
                    trait_ref: Some(def(103)),
                    entries: vec![
                        VtableEntry::Drop(None),
                        VtableEntry::Size(8),
                        VtableEntry::Align(4),
                        VtableEntry::Method(crate::jir::semantic::DirectItemRef {
                            item: item(2),
                            link_name: crate::jir::interns::StrId(0),
                        }),
                    ],
                },
                semantic: BodySemantic::Ordinary,
                signature: sig(Vec::new(), UNIT),
                locals: Vec::new(),
                blocks: Vec::new(),
            }),
            &["method"],
        );
        let graph = prog.collect();
        let receiver_key = value_node("method", Site::Value(receiver), &[Step::Deref]);
        assert!(reasons_of(&graph, &receiver_key).contains(&InfReason::UnknownCall));
        let group = NodeKey::Group {
            key: GroupKey::Vtable {
                trait_ref: Some(def(103)),
                slot: 3,
            },
            slot: BoundarySlot::Param(1),
            path: Vec::new(),
        };
        assert!(graph.edges.contains(&Edge {
            a: value_node("method", Site::Value(arg), &[]),
            b: group.clone(),
            kind: EdgeKind::Shared,
        }));
        // A complete vtable group is not pinned.
        assert!(reasons_of(&graph, &group).is_empty(), "{}", graph.dump());
        // The method is modeled through its group, not the unlowered-refs pin.
        let arg_key = value_node("method", Site::Value(arg), &[]);
        assert!(!reasons_of(&graph, &arg_key).contains(&InfReason::UnknownCall));
    }

    #[test]
    fn a_fn_pointer_group_collapses_targets_and_pins_its_slots() {
        let mut prog = Prog::new();
        prog.add("target", identity_func(90, STRING, STRING));
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let s = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let f = b.op(entry, OpKind::Const(ConstTree::Unit), FN_DEF, NO_ORIGIN);
        let fp = b.op(
            entry,
            OpKind::Cast {
                kind: CastKind::PointerCoercion,
                v: f,
                from: FN_DEF,
                to: FNPTR,
            },
            FNPTR,
            NO_ORIGIN,
        );
        b.effect(
            entry,
            OpKind::Call {
                callee: Callee::Indirect { signature: FNPTR },
                function: Some(fp),
                args: vec![s],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("caller", b.finish());
        let graph = prog.collect();
        let Ok(target) = &prog.funcs[&JsName::new("target".to_owned())] else {
            unreachable!()
        };
        let group = NodeKey::Group {
            key: GroupKey::FnPtr(FNPTR),
            slot: BoundarySlot::Param(0),
            path: Vec::new(),
        };
        // The target joins the group, the call site edges into it, and the group pins because
        // reifications inside unlowered items cannot be enumerated.
        assert!(graph.edges.contains(&Edge {
            a: value_node("target", Site::Value(target.params[0]), &[]),
            b: group.clone(),
            kind: EdgeKind::Shared,
        }));
        assert!(graph
            .edges
            .iter()
            .any(|edge| { edge.b == group && matches!(edge.kind, EdgeKind::Value { .. }) }));
        assert!(reasons_of(&graph, &group).contains(&InfReason::UnknownCall));
    }

    #[test]
    fn a_static_reference_shares_with_a_pinned_static_node() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let r = b.op(
            entry,
            OpKind::Const(ConstTree::Global {
                target: GlobalReference::Static {
                    definition: def(60),
                    link_name: crate::jir::interns::StrId(0),
                },
                offset: 0,
            }),
            REF_STRING,
            NO_ORIGIN,
        );
        let _ = r;
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let static_node = NodeKey::Static {
            definition: def(60),
            path: vec![Step::Deref],
        };
        assert!(reasons_of(&graph, &static_node).contains(&InfReason::StaticEscape));
        assert!(graph.edges.contains(&Edge {
            a: value_node("f", Site::Value(r), &[Step::Deref]),
            b: static_node,
            kind: EdgeKind::Shared,
        }));
    }

    #[test]
    fn export_round_trips_connect_matching_slots_with_edges() {
        let mut prog = Prog::new();
        prog.add("take", identity_func(1, STRING, UNIT));
        prog.add("give", identity_func(2, STRING, STRING));
        prog.exported("take");
        prog.exported("give");
        let graph = prog.collect();
        let exports: Vec<&Edge> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Export)
            .collect();
        assert!(!exports.is_empty(), "{}", graph.dump());
    }

    #[test]
    fn an_item_named_outside_a_direct_call_pins_its_boundary_only() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(2), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        // An internal value that never reaches the boundary stays free.
        let local = b.op(entry, OpKind::Const(ConstTree::Unit), STRING, NO_ORIGIN);
        let _ = local;
        b.terminate(entry, RTerm::Return(None));
        prog.add("named", b.finish());
        prog.add_unlowered("baseline_text", None, &["named"]);
        let graph = prog.collect();
        let param = value_node("named", Site::Value(p), &[]);
        assert!(reasons_of(&graph, &param).contains(&InfReason::UnknownCall));
        let local_key = value_node("named", Site::Value(local), &[]);
        assert!(reasons_of(&graph, &local_key).is_empty());
    }

    /// An exported item's candidate boundary slots are recorded for the sidecar and pinned until
    /// export wrappers can adapt carriers. Interior values of the same item are neither recorded
    /// nor pinned.
    #[test]
    fn an_exported_items_candidate_slots_are_published_and_pinned() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], STRING));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let local = b.op(entry, OpKind::Const(ConstTree::Unit), STRING, NO_ORIGIN);
        let _ = local;
        b.terminate(entry, RTerm::Return(Some(p)));
        prog.add("give", b.finish());
        prog.exported("give");
        let graph = prog.collect();
        let param = value_node("give", Site::Value(p), &[]);
        assert!(reasons_of(&graph, &param).contains(&InfReason::Exported));
        assert!(graph.exported_slots.contains(&param));
        let ret = value_node("give", Site::Ret, &[]);
        assert!(reasons_of(&graph, &ret).contains(&InfReason::Exported));
        assert!(graph.exported_slots.contains(&ret));
        let local_key = value_node("give", Site::Value(local), &[]);
        assert!(reasons_of(&graph, &local_key).is_empty());
        assert!(!graph.exported_slots.contains(&local_key));
    }

    /// The live set filters the table-driven boundary walks: an unreachable baseline item's
    /// references pin nothing, so a dead body cannot keep a live value baseline.
    #[test]
    fn an_unreachable_items_references_pin_nothing() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.terminate(entry, RTerm::Return(None));
        prog.add("kept", b.finish());
        // Unlowered baseline text naming `kept`: live, it pins kept's boundary
        // (`an_item_named_outside_a_direct_call_pins_its_boundary_only`); dead, it must not.
        prog.add_unlowered("dead_text", None, &["kept"]);
        let interns: BTreeMap<JsName, &Interns> = prog
            .table
            .keys()
            .map(|name| (name.clone(), &prog.interns))
            .collect();
        let live: BTreeSet<JsName> = [JsName::new("kept".to_owned())].into_iter().collect();
        let graph = collect(&prog.table, &prog.funcs, &interns, &live);
        let param = value_node("kept", Site::Value(p), &[]);
        assert!(
            !reasons_of(&graph, &param).contains(&InfReason::UnknownCall),
            "{}",
            graph.dump()
        );
        // The same program with the text live pins the boundary.
        let graph = prog.collect();
        assert!(reasons_of(&graph, &param).contains(&InfReason::UnknownCall));
    }

    /// The D2 cut, mutable-receiver class (`VecPush`): a registry-covered `Standard` call to a
    /// lowered callee makes NO deref-crossing edge — even a raw-view-poisoned Ordinary body
    /// cannot reach the caller — while the caller's own receiver picks up the requirement's
    /// mutability, carrier set, and the op's element flow, and by-value slots keep their priced
    /// adapter edge.
    #[test]
    fn a_registry_covered_standard_call_cuts_the_deref_join_and_constrains_the_caller() {
        let mut prog = Prog::new();
        // An Ordinary callee (the `push_mut`-shaped internal) whose body raw-views its receiver.
        let mut b = FuncBuilder::new(item(2), sig(vec![MUT_VEC, STRING], UNIT));
        let receiver = b.param(MUT_VEC, NO_ORIGIN);
        let value = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.op(
            entry,
            OpKind::RawView {
                root: Root::Value(receiver),
                path: Vec::new(),
                mutable: true,
            },
            MUT_VEC,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("push_like", b.finish());
        let mut c = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let v = c.op(entry, OpKind::Const(ConstTree::Unit), VEC, NO_ORIGIN);
        let r = c.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(v),
                path: Vec::new(),
                mutable: true,
            },
            MUT_VEC,
            NO_ORIGIN,
        );
        let s = c.op(entry, OpKind::Const(ConstTree::Unit), STRING, NO_ORIGIN);
        c.effect(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![r, s],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecPush),
                host: None,
            },
            NO_ORIGIN,
        );
        c.terminate(entry, RTerm::Return(None));
        prog.add("caller", c.finish());
        let graph = prog.collect();
        // No edge of any kind crosses from the caller into the callee's deref paths.
        for edge in &graph.edges {
            let deref = |key: &NodeKey| matches!(key, NodeKey::Value { path, .. } if path.contains(&Step::Deref));
            let of = |key: &NodeKey| match key {
                NodeKey::Value { item, .. } => item.as_str().to_owned(),
                _ => String::new(),
            };
            if of(&edge.a) != of(&edge.b) {
                assert!(
                    !deref(&edge.a) && !deref(&edge.b),
                    "a deref path crossed the cut: {} - {}",
                    edge.a,
                    edge.b
                );
            }
        }
        // The caller's receiver: mutable, unpoisoned, constrained to the requirement's carriers.
        let near = value_node("caller", Site::Value(r), &[Step::Deref]);
        assert!(graph.nodes[&near].mutable);
        assert!(reasons_of(&graph, &near).is_empty(), "{}", graph.dump());
        assert_eq!(
            graph.nodes[&near].carriers,
            Some([crate::jir::repr::Carrier::NativeVec].into_iter().collect())
        );
        // The op's element flow runs caller-side: pushed value = receiver element.
        let element = value_node("caller", Site::Value(r), &[Step::Deref, Step::Element]);
        let pushed = value_node("caller", Site::Value(s), &[]);
        let pair = if element <= pushed {
            (element, pushed)
        } else {
            (pushed, element)
        };
        assert!(graph.unions.contains(&pair), "{}", graph.dump());
        // The by-value slot keeps its adapter edge into the callee.
        assert!(graph.edges.contains(&Edge {
            a: value_node("caller", Site::Value(s), &[]),
            b: value_node("push_like", Site::Value(value), &[]),
            kind: EdgeKind::Value { source_dead: true },
        }));
        // The callee's own poison stays its own.
        let far = value_node("push_like", Site::Value(receiver), &[Step::Deref]);
        assert!(reasons_of(&graph, &far).contains(&InfReason::RawProvenance));
    }

    /// The result class (`VecNew`): the cut keeps the by-value result edge (mixing there is an
    /// adapter, exactly as before) and constrains the caller's result node.
    #[test]
    fn a_covered_standard_result_keeps_its_value_edge_and_is_constrained() {
        let mut prog = Prog::new();
        prog.add("vec_new", identity_func(2, UNIT, VEC));
        let mut c = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let unit = c.op(entry, OpKind::Const(ConstTree::Unit), UNIT, NO_ORIGIN);
        let made = c.op(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![unit],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecNew),
                host: None,
            },
            VEC,
            NO_ORIGIN,
        );
        let _ = made;
        c.terminate(entry, RTerm::Return(None));
        prog.add("caller", c.finish());
        let graph = prog.collect();
        let result = value_node("caller", Site::Value(made), &[]);
        assert!(graph.edges.contains(&Edge {
            a: value_node("vec_new", Site::Ret, &[]),
            b: result.clone(),
            kind: EdgeKind::Value { source_dead: false },
        }));
        assert_eq!(
            graph.nodes[&result].carriers,
            Some([crate::jir::repr::Carrier::NativeVec].into_iter().collect())
        );
    }

    /// An UNCOVERED standard operation's call still joins like an ordinary call: the cut fires
    /// only on registry-covered operations.
    #[test]
    fn an_uncovered_standard_call_still_joins_like_an_ordinary_call() {
        let mut prog = Prog::new();
        prog.add("pop_like", identity_func(2, MUT_VEC, UNIT));
        let mut c = FuncBuilder::new(item(1), sig(vec![MUT_VEC], UNIT));
        let r = c.param(MUT_VEC, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        c.effect(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![r],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecPop),
                host: None,
            },
            NO_ORIGIN,
        );
        c.terminate(entry, RTerm::Return(None));
        prog.add("caller", c.finish());
        let graph = prog.collect();
        let Ok(callee) = &prog.funcs[&JsName::new("pop_like".to_owned())] else {
            unreachable!()
        };
        assert!(graph.edges.contains(&Edge {
            a: value_node("caller", Site::Value(r), &[Step::Deref]),
            b: value_node("pop_like", Site::Value(callee.params[0]), &[Step::Deref]),
            kind: EdgeKind::Shared,
        }));
    }

    /// The registry-described flows never pin the receiver: a summarized `VecPush` over a
    /// non-candidate element leaves the receiver's own node clean (the fixture-37 poison).
    #[test]
    fn a_summarized_flow_over_a_non_candidate_element_does_not_pin_the_receiver() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(2), sig(vec![MUT_VEC, U32], UNIT));
        let receiver = b.param(MUT_VEC, NO_ORIGIN);
        let _value = b.param(U32, NO_ORIGIN);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add_semantic(
            "push",
            b.finish(),
            BodySemantic::Standard(StandardOperation::VecPush),
        );
        let graph = prog.collect();
        let key = value_node("push", Site::Value(receiver), &[Step::Deref]);
        assert!(reasons_of(&graph, &key).is_empty(), "{}", graph.dump());
        assert!(graph.nodes[&key].mutable);
    }

    #[test]
    fn collection_is_deterministic() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        prog.add(
            "caller",
            call_func(1, STRING, direct(2), CallSemantic::Ordinary, None),
        );
        prog.add("give", identity_func(3, VEC, VEC));
        prog.exported("give");
        let first = prog.collect().dump();
        let second = prog.collect().dump();
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    // -- the W5f fresh-allocation box-view whitelist ------------------------------------------

    /// `*mut u32`.
    const RAW: TypeId = TypeId(40);
    /// A second thin raw pointer, the init view's type.
    const RAW2: TypeId = TypeId(41);
    /// A candidate `Box`.
    const BOXED: TypeId = TypeId(42);
    /// A different candidate `Box`.
    const BOXED2: TypeId = TypeId(43);

    fn add_raw_types(prog: &mut Prog) {
        use crate::jir::types::{
            AllocatorIdentity, NominalAbi, PinnedBoundary, RepresentationCandidate, TypeMeta,
        };
        let box_meta = TypeMeta {
            candidate: RepresentationCandidate::Box,
            abi: NominalAbi::Rust,
            boundary: PinnedBoundary::None,
            allocator: AllocatorIdentity::Global,
            iterator: crate::jir::types::IteratorIdentity::None,
        };
        let boxed = |id: TypeId, def_hash: u64| {
            descriptor(
                id,
                TypeKind::Adt {
                    def: def(def_hash),
                    args: Vec::new(),
                    form: NominalForm::Struct,
                    variants: Vec::new(),
                    niche: None,
                },
                box_meta,
                sized(),
            )
        };
        for d in [
            descriptor(
                RAW,
                TypeKind::RawPointer {
                    pointee: U32,
                    mutable: true,
                },
                TypeMeta::default(),
                sized(),
            ),
            descriptor(
                RAW2,
                TypeKind::RawPointer {
                    pointee: UNIT,
                    mutable: true,
                },
                TypeMeta::default(),
                sized(),
            ),
            boxed(BOXED, 140),
            boxed(BOXED2, 141),
        ] {
            assert!(prog.interns.types.insert(d));
        }
    }

    /// The allocation entry point every whitelist test calls: name-matched, `RawParts`.
    const ALLOC_CALLEE: &str = "alloc$boxed$box_new_uninit$h0";

    fn alloc_call(b: &mut FuncBuilder, callee: u64) -> ValueId {
        let entry = FuncBuilder::ENTRY;
        let size = b.op(entry, OpKind::Const(ConstTree::Unit), U32, NO_ORIGIN);
        b.op(
            entry,
            OpKind::Call {
                callee: direct(callee),
                function: None,
                args: vec![size],
                track_caller: None,
                semantic: CallSemantic::RawParts,
                host: None,
            },
            RAW,
            NO_ORIGIN,
        )
    }

    fn transmute_to(b: &mut FuncBuilder, v: ValueId, from: TypeId, to: TypeId) -> ValueId {
        b.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::Transmute,
                v,
                from,
                to,
            },
            to,
            NO_ORIGIN,
        )
    }

    #[test]
    fn a_fresh_allocation_transmuted_to_one_box_type_is_not_raw_pinned() {
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let p = alloc_call(&mut b, 50);
        // The init write of the inlined `Rc::new` shape: a raw re-typing, a reborrow, a
        // candidate-free store.
        let q = b.op(
            entry,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: p,
                from: RAW,
                to: RAW2,
            },
            RAW2,
            NO_ORIGIN,
        );
        let r = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(q),
                path: vec![Proj::Deref],
                mutable: true,
            },
            RAW2,
            NO_ORIGIN,
        );
        let value = b.op(entry, OpKind::Const(ConstTree::Unit), U32, NO_ORIGIN);
        b.effect(entry, OpKind::StoreThrough { ptr: r, v: value }, NO_ORIGIN);
        let first = transmute_to(&mut b, p, RAW, BOXED);
        let second = transmute_to(&mut b, p, RAW, BOXED);
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        for view in [first, second] {
            assert!(
                !reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
                    .contains(&InfReason::RawProvenance),
                "{}",
                graph.dump()
            );
        }
        // The sibling views alias one allocation and answer as one node.
        assert!(graph.unions.contains(&(
            value_node("f", Site::Value(first), &[]),
            value_node("f", Site::Value(second), &[]),
        )));
    }

    #[test]
    fn a_box_consumed_only_by_its_pointer_field_is_not_projection_pinned() {
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let allocation = alloc_call(&mut b, 50);
        let boxed = transmute_to(&mut b, allocation, RAW, BOXED);
        let _pointer = b.op(
            entry,
            OpKind::Field {
                base: boxed,
                variant: None,
                field: 0,
            },
            RAW,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(
            !reasons_of(&graph, &value_node("f", Site::Value(boxed), &[]))
                .contains(&InfReason::UnsupportedOperation),
            "{}",
            graph.dump()
        );
    }

    #[test]
    fn a_box_pointer_projection_with_another_owner_use_stays_pinned() {
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let allocation = alloc_call(&mut b, 50);
        let boxed = transmute_to(&mut b, allocation, RAW, BOXED);
        let _pointer = b.op(
            entry,
            OpKind::Field {
                base: boxed,
                variant: None,
                field: 0,
            },
            RAW,
            NO_ORIGIN,
        );
        let _second_use = b.op(entry, OpKind::Move(boxed), BOXED, NO_ORIGIN);
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(
            reasons_of(&graph, &value_node("f", Site::Value(boxed), &[]))
                .contains(&InfReason::UnsupportedOperation),
            "{}",
            graph.dump()
        );
    }

    #[test]
    fn sibling_views_at_two_box_types_are_admitted_and_unioned() {
        // `Rc::new`'s real shape: `Box<MaybeUninit<T>>` for the init write beside the final
        // `Box<T>` — one storage, one node.
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let p = alloc_call(&mut b, 50);
        let first = transmute_to(&mut b, p, RAW, BOXED);
        let second = transmute_to(&mut b, p, RAW, BOXED2);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        for view in [first, second] {
            assert!(
                !reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
                    .contains(&InfReason::RawProvenance)
            );
        }
        assert!(graph.unions.contains(&(
            value_node("f", Site::Value(first), &[]),
            value_node("f", Site::Value(second), &[]),
        )));
    }

    #[test]
    fn a_non_box_transmute_target_refuses_the_whitelist() {
        // A pointer-to-integer transmute exposes the address.
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let p = alloc_call(&mut b, 50);
        let _exposed = transmute_to(&mut b, p, RAW, U32);
        let view = transmute_to(&mut b, p, RAW, BOXED);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
            .contains(&InfReason::RawProvenance));
    }

    #[test]
    fn an_unknown_producer_refuses_the_whitelist() {
        // `as_ptr`-shaped: the pointer views storage that already has other names.
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add("app$buffer$as_ptr$h0", identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let p = alloc_call(&mut b, 50);
        let view = transmute_to(&mut b, p, RAW, BOXED);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
            .contains(&InfReason::RawProvenance));
    }

    #[test]
    fn an_address_comparison_refuses_the_whitelist() {
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let p = alloc_call(&mut b, 50);
        let _cmp = b.op(
            entry,
            OpKind::Binary {
                op: crate::jir::semantic::BinaryOp::Eq,
                l: p,
                r: p,
                ty: RAW,
            },
            U32,
            NO_ORIGIN,
        );
        let view = transmute_to(&mut b, p, RAW, BOXED);
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
            .contains(&InfReason::RawProvenance));
    }

    #[test]
    fn an_escape_into_a_call_refuses_the_whitelist() {
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        prog.add("observer", identity_func(51, RAW, UNIT));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let p = alloc_call(&mut b, 50);
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(51),
                function: None,
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        let view = transmute_to(&mut b, p, RAW, BOXED);
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
            .contains(&InfReason::RawProvenance));
    }

    #[test]
    fn a_candidate_typed_write_through_the_pointer_refuses_the_whitelist() {
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let p = alloc_call(&mut b, 50);
        let value = b.op(entry, OpKind::Const(ConstTree::Unit), STRING, NO_ORIGIN);
        b.effect(entry, OpKind::StoreThrough { ptr: p, v: value }, NO_ORIGIN);
        let view = transmute_to(&mut b, p, RAW, BOXED);
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
            .contains(&InfReason::RawProvenance));
    }

    #[test]
    fn a_terminator_use_refuses_the_whitelist() {
        let mut prog = Prog::new();
        add_raw_types(&mut prog);
        prog.add(ALLOC_CALLEE, identity_func(50, U32, RAW));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let p = alloc_call(&mut b, 50);
        let view = transmute_to(&mut b, p, RAW, BOXED);
        let next = b.block();
        let _q = b.block_param(next, RAW, NO_ORIGIN);
        b.terminate(entry, RTerm::Goto(next, vec![p]));
        b.terminate(next, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        assert!(reasons_of(&graph, &value_node("f", Site::Value(view), &[]))
            .contains(&InfReason::RawProvenance));
    }
}
