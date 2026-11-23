//! Legality terms over the value graph: what a native carrier may never touch, spelled so that
//! nothing ever poisons a whole component.
//!
//! v1's representation planner answered legality with one `BaselineReason` on a connected
//! component: any pin anywhere kept every value it flowed near baseline, which is why one opaque
//! host handle in a function kept every string in it baseline. This module keeps v1's *facts* and
//! changes their SHAPE: every fact becomes one of exactly three term kinds, each local to one
//! node or one edge of [`crate::analysis::valueflow::ValueGraph`], and the min-cut solver (W4b)
//! is what weighs them against each other:
//!
//! * [`Term::UnaryInf`]: the node itself can never be native (raw provenance, `repr(C)`, a pinned
//!   view ABI, an unprovable allocator, an unsupported operation, ...).
//! * [`Term::MustEqual`]: the two endpoints must choose one label, because they share storage or an
//!   observer sees both (a borrow crossing a call, an export round trip, a host slot whose contract
//!   retains and mutates). An edge to [`NodeKey::Baseline`] pins its one endpoint and nothing else.
//! * [`Term::Adapter`]: the endpoints may differ at a conversion cost. The cost is directional:
//!   native to baseline always has a materialization spelling, baseline to native is legal only
//!   where the source value is dead and unaliased after the edge.
//!
//! The cost of a finite term is not decided here: the cost model (`crate::plan::cost`) prices the
//! adapters and the solver prices the cut. Legality only says which arcs are infinite.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::valueflow::{CandidateKind, EdgeKind, NodeKey, Step, UnionFind, ValueGraph};
use crate::descriptor::{Fact, SlotFacts};
use crate::jir::repr::Carrier;
/// The v1 representation planner's complete reason catalog, kept as documentation now that the
/// planner itself is deleted: [`v1_term`] and its table test record what each reason became in
/// the v2 vocabulary.
#[allow(dead_code)] // The v1-reason table test is the consumer; the mapping is the contract.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BaselineReason {
    BodyUnavailable,
    HostBoundary,
    ViewAbiBoundary,
    CRepresentation,
    ForeignCall,
    UnknownCall,
    RawProvenance,
    CustomAllocator,
    UnknownAllocator,
    StaticEscape,
    UserDefinedDrop,
    UnsupportedOperation,
    Unsized,
    ZeroSized,
    UnknownLayout,
    MixedRepresentations,
}

/// Why one node can never take a native carrier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum InfReason {
    /// The callee's typed body is unavailable, so nothing can be proven about the slot.
    BodyUnavailable,
    /// The value crosses the pinned view ABI.
    ViewAbi,
    /// `repr(C)` fixes the representation.
    CAbi,
    /// A foreign call can observe the baseline representation.
    ForeignCall,
    /// An indirect, dynamic, or otherwise unresolved caller can observe the representation.
    UnknownCall,
    /// Raw provenance or byte access can observe the representation.
    RawProvenance,
    /// A custom allocator can observe the allocation.
    CustomAllocator,
    /// The artifact allocator could not be proven to be the default.
    UnknownAllocator,
    /// The value lives in (or shares storage with) a static.
    StaticEscape,
    /// User-defined drop behavior observes the baseline representation.
    UserDefinedDrop,
    /// An operation no registry entry covers touches the value.
    UnsupportedOperation,
    /// The represented value (or a vec element) is unsized.
    Unsized,
    /// Zero-sized values keep their baseline representation.
    ZeroSized,
    /// Element size or alignment is unavailable.
    UnknownLayout,
    /// One value node is reached as two incompatible candidate kinds.
    MixedRepresentations,
    /// The value sits in an exported item's parameter or return slot. Export wrappers do not yet
    /// adapt native carriers at the JavaScript boundary, so the public ABI remains baseline.
    Exported,
}

impl InfReason {
    /// The name the report spells the reason by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BodyUnavailable => "body-unavailable",
            Self::ViewAbi => "view-abi",
            Self::CAbi => "c-representation",
            Self::ForeignCall => "foreign-call",
            Self::UnknownCall => "unknown-call",
            Self::RawProvenance => "raw-provenance",
            Self::CustomAllocator => "custom-allocator",
            Self::UnknownAllocator => "unknown-allocator",
            Self::StaticEscape => "static-escape",
            Self::UserDefinedDrop => "user-defined-drop",
            Self::UnsupportedOperation => "unsupported-operation",
            Self::Unsized => "unsized",
            Self::ZeroSized => "zero-sized",
            Self::UnknownLayout => "unknown-layout",
            Self::MixedRepresentations => "mixed-representations",
            Self::Exported => "exported",
        }
    }
}

/// Why two endpoints must choose one label.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EqualReason {
    /// The endpoints share storage through a pointer, so a converted copy on one side would
    /// desynchronize the other.
    SharedStorage,
    /// A host slot whose contract observes what a carrier would change; the far endpoint is
    /// [`NodeKey::Baseline`].
    HostPin,
    /// JavaScript may return one export's value into another, so the slots of one export
    /// component answer as one.
    ExportRoundTrip,
}

/// One legality term: a fact about one node or one edge, never about a component.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Term {
    /// `node` can never be native.
    UnaryInf { node: NodeKey, reason: InfReason },
    /// `a` and `b` must take the same label.
    MustEqual {
        a: NodeKey,
        b: NodeKey,
        reason: EqualReason,
    },
    /// `a` and `b` may differ at a directional adapter cost: `a` native beside `b` baseline is
    /// always priceable, `a` baseline beside `b` native only when the source value was proven
    /// dead and unaliased after the edge.
    Adapter {
        a: NodeKey,
        b: NodeKey,
        baseline_to_native_legal: bool,
    },
    /// `node` may go native only at exactly `carrier`: a must-equal against a fixed-carrier
    /// anchor. Produced by a host sink slot whose declared accepts-set admits one of the node's
    /// solvable carriers; the family sub-solves read it off the problem node.
    FixedCarrier { node: NodeKey, carrier: Carrier },
}

/// The shape of a term, for mapping v1's component reasons onto the vocabulary.
#[allow(dead_code)] // The v1-reason table test is the consumer; the mapping is the contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TermKind {
    UnaryInf,
    MustEqual,
    Adapter,
}

/// The term a v1 `BaselineReason` becomes.
///
/// Every reason the v1 planner could poison a component with maps to exactly one kind here. The
/// one deliberate correction (D2): `HostBoundary` was a whole-item blocker in v1
/// (`representation_plan.rs::collect_host_call`) and is a must-equal edge on the one pinned slot
/// in v2.
#[allow(dead_code)] // The v1-reason table test is the consumer; the mapping is the contract.
pub fn v1_term(reason: BaselineReason) -> (TermKind, Option<InfReason>) {
    let unary = |reason| (TermKind::UnaryInf, Some(reason));
    match reason {
        BaselineReason::BodyUnavailable => unary(InfReason::BodyUnavailable),
        BaselineReason::HostBoundary => (TermKind::MustEqual, None),
        BaselineReason::ViewAbiBoundary => unary(InfReason::ViewAbi),
        BaselineReason::CRepresentation => unary(InfReason::CAbi),
        BaselineReason::ForeignCall => unary(InfReason::ForeignCall),
        BaselineReason::UnknownCall => unary(InfReason::UnknownCall),
        BaselineReason::RawProvenance => unary(InfReason::RawProvenance),
        BaselineReason::CustomAllocator => unary(InfReason::CustomAllocator),
        BaselineReason::UnknownAllocator => unary(InfReason::UnknownAllocator),
        BaselineReason::StaticEscape => unary(InfReason::StaticEscape),
        BaselineReason::UserDefinedDrop => unary(InfReason::UserDefinedDrop),
        BaselineReason::UnsupportedOperation => unary(InfReason::UnsupportedOperation),
        BaselineReason::Unsized => unary(InfReason::Unsized),
        BaselineReason::ZeroSized => unary(InfReason::ZeroSized),
        BaselineReason::UnknownLayout => unary(InfReason::UnknownLayout),
        BaselineReason::MixedRepresentations => unary(InfReason::MixedRepresentations),
    }
}

/// The legality terms of `graph`, sorted and deduplicated.
///
/// A sink edge ([`EdgeKind::HostSink`]) is free exactly for the carriers its declaration
/// accepts: [`host_sink_term`] leaves no term where every solvable carrier is accepted, fixes
/// the slot's value where exactly one is, and pins it where none is.
pub fn terms(graph: &ValueGraph) -> Vec<Term> {
    let merged = merged_facts(graph);
    let mut terms = BTreeSet::new();
    for (node, facts) in &graph.nodes {
        for reason in &facts.reasons {
            terms.insert(Term::UnaryInf {
                node: node.clone(),
                reason: *reason,
            });
        }
    }
    // A union joins two names for one value; two candidate kinds under one name is type
    // confusion the plan must never label, spelled as a unary term on the merged node's
    // representative rather than any component-shaped kill.
    for node in mixed_nodes(graph) {
        terms.insert(Term::UnaryInf {
            node,
            reason: InfReason::MixedRepresentations,
        });
    }
    // A registry-covered standard call's slot requirement constrains the carriers the node may
    // take (the D2 cut's caller-side legality). Against the carriers the family sub-solves can
    // choose: all accepted leaves the edge free, exactly one fixes it, none pins — the same
    // three-way answer a host sink's accepts-set gets.
    for (node, facts) in &graph.nodes {
        let Some(constraint) = &facts.carriers else {
            continue;
        };
        let (kind, mutable) = merged.get(node).copied().unwrap_or((None, false));
        let solvable: &[Carrier] = match kind {
            Some(CandidateKind::String) if mutable => &[Carrier::StringBuilder],
            Some(CandidateKind::String) => &[Carrier::HostString, Carrier::StringBuilder],
            Some(CandidateKind::Vec) => &[Carrier::NativeVec],
            Some(CandidateKind::Box) => &[Carrier::NativeBox],
            None => continue,
        };
        let allowed: Vec<Carrier> = solvable
            .iter()
            .copied()
            .filter(|carrier| constraint.contains(carrier))
            .collect();
        if allowed.len() == solvable.len() {
            continue;
        }
        terms.insert(match allowed[..] {
            [carrier] => Term::FixedCarrier {
                node: node.clone(),
                carrier,
            },
            _ => Term::MustEqual {
                a: node.clone(),
                b: NodeKey::Baseline,
                reason: EqualReason::HostPin,
            },
        });
    }
    for edge in &graph.edges {
        let term = match edge.kind {
            EdgeKind::Shared => Some(Term::MustEqual {
                a: edge.a.clone(),
                b: edge.b.clone(),
                reason: EqualReason::SharedStorage,
            }),
            EdgeKind::HostPin => Some(Term::MustEqual {
                a: edge.a.clone(),
                b: edge.b.clone(),
                reason: EqualReason::HostPin,
            }),
            EdgeKind::Export => Some(Term::MustEqual {
                a: edge.a.clone(),
                b: edge.b.clone(),
                reason: EqualReason::ExportRoundTrip,
            }),
            EdgeKind::Value { source_dead } => Some(Term::Adapter {
                a: edge.a.clone(),
                b: edge.b.clone(),
                baseline_to_native_legal: source_dead,
            }),
            EdgeKind::HostSink { accepts } => host_sink_term(&edge.a, accepts, &merged),
        };
        terms.extend(term);
    }
    terms.into_iter().collect()
}

/// The term a host sink edge leaves on the slot's value, if any.
///
/// The declaration's accepts-set speaks for the slot's own value — the node reached through
/// derefs alone; a candidate nested deeper is a shape the declaration never named. Against the
/// carriers the family sub-solves can actually choose (`StringBuilder` is never declarable, and
/// a mutable string can only be a builder):
///
/// * every solvable carrier accepted → the edge is free (no term);
/// * exactly one accepted → the node may go native only at that carrier ([`Term::FixedCarrier`]);
/// * none accepted → the node pins, exactly as if the slot had declared no carrier at all.
fn host_sink_term(
    node: &NodeKey,
    accepts: SlotFacts,
    merged: &BTreeMap<NodeKey, (Option<CandidateKind>, bool)>,
) -> Option<Term> {
    let (kind, mutable) = merged.get(node).copied().unwrap_or((None, false));
    let solvable: &[Carrier] = match kind {
        Some(CandidateKind::String) if mutable => &[Carrier::StringBuilder],
        Some(CandidateKind::String) => &[Carrier::HostString, Carrier::StringBuilder],
        Some(CandidateKind::Vec) => &[Carrier::NativeVec],
        Some(CandidateKind::Box) => &[Carrier::NativeBox],
        // A kindless node has nothing to fix.
        None => return None,
    };
    let deref_only = match node {
        NodeKey::Value { path, .. } | NodeKey::Opaque { path, .. } => {
            path.iter().all(|step| matches!(step, Step::Deref))
        }
        _ => false,
    };
    let accepted = |carrier: Carrier| {
        deref_only
            && match carrier {
                Carrier::HostString => accepts.has(Fact::HostString),
                Carrier::NativeVec => accepts.has(Fact::NativeVec),
                Carrier::SliceView => accepts.has(Fact::SliceView),
                _ => false,
            }
    };
    if solvable.iter().all(|carrier| accepted(*carrier)) {
        return None;
    }
    let allowed: Vec<Carrier> = solvable
        .iter()
        .copied()
        .filter(|carrier| accepted(*carrier))
        .collect();
    match allowed[..] {
        [carrier] => Some(Term::FixedCarrier {
            node: node.clone(),
            carrier,
        }),
        _ => Some(Term::MustEqual {
            a: node.clone(),
            b: NodeKey::Baseline,
            reason: EqualReason::HostPin,
        }),
    }
}

/// The union-merged (candidate kind, mutability) each node answers with: a value's family or
/// mutation may be recorded under any name its union gave it, and a sink decision made from one
/// name's facts alone could fix a mutable string to `HostString`.
fn merged_facts(graph: &ValueGraph) -> BTreeMap<NodeKey, (Option<CandidateKind>, bool)> {
    let keys: Vec<&NodeKey> = graph.nodes.keys().collect();
    let index: BTreeMap<&NodeKey, usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let mut unions = UnionFind::new(keys.len());
    for (a, b) in &graph.unions {
        if let (Some(a), Some(b)) = (index.get(a), index.get(b)) {
            unions.union(*a, *b);
        }
    }
    let mut roots: BTreeMap<usize, (Option<CandidateKind>, bool)> = BTreeMap::new();
    for (i, key) in keys.iter().enumerate() {
        let facts = &graph.nodes[*key];
        let entry = roots.entry(unions.root(i)).or_insert((None, false));
        entry.0 = entry.0.or(facts.kind);
        entry.1 |= facts.mutable;
    }
    let mut merged = BTreeMap::new();
    for (i, key) in keys.iter().enumerate() {
        merged.insert((*key).clone(), roots[&unions.root(i)]);
    }
    merged
}

/// The representative of every union whose members disagree about their candidate kind.
fn mixed_nodes(graph: &ValueGraph) -> Vec<NodeKey> {
    let keys: Vec<&NodeKey> = graph.nodes.keys().collect();
    let index: std::collections::BTreeMap<&NodeKey, usize> =
        keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let mut unions = UnionFind::new(keys.len());
    for (a, b) in &graph.unions {
        if let (Some(a), Some(b)) = (index.get(a), index.get(b)) {
            unions.union(*a, *b);
        }
    }
    let mut kind_of = std::collections::BTreeMap::new();
    let mut mixed = BTreeSet::new();
    for (i, key) in keys.iter().enumerate() {
        let Some(kind) = graph.nodes[*key].kind else {
            continue;
        };
        let root = unions.root(i);
        match kind_of.get(&root) {
            None => {
                kind_of.insert(root, kind);
            }
            Some(existing) if *existing != kind => {
                mixed.insert(root);
            }
            Some(_) => {}
        }
    }
    mixed.into_iter().map(|root| keys[root].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::valueflow::harness::*;
    use crate::analysis::valueflow::{
        CandidateKind, Edge, EdgeKind, NodeFacts, Site, Step, ValueGraph,
    };
    use crate::descriptor::{Descriptor, Fact, Shape, Slot, SlotFacts, PREFIX};
    use crate::jir::interns::StrId;
    use crate::jir::repr::{FuncBuilder, OpKind, RTerm, Root};
    use crate::jir::semantic::{
        BodySemantic, CallSemantic, Callee, ConstTree, GlobalReference, HostPayload,
        StandardOperation,
    };
    use crate::jir::types::{AllocatorIdentity, NominalAbi, PinnedBoundary};
    use crate::jir::TypeId;

    /// The terms whose subject involves `key`.
    fn terms_on(terms: &[Term], key: &NodeKey) -> Vec<Term> {
        terms
            .iter()
            .filter(|term| match term {
                Term::UnaryInf { node, .. } | Term::FixedCarrier { node, .. } => node == key,
                Term::MustEqual { a, b, .. } | Term::Adapter { a, b, .. } => a == key || b == key,
            })
            .cloned()
            .collect()
    }

    fn has_unary(terms: &[Term], key: &NodeKey, reason: InfReason) -> bool {
        terms.iter().any(|term| {
            matches!(term, Term::UnaryInf { node, reason: r } if node == key && *r == reason)
        })
    }

    #[test]
    fn every_v1_baseline_reason_maps_to_exactly_one_term_kind() {
        // The full v1 catalog (`representation_plan.rs::BaselineReason`), each with the kind its
        // fact takes in the edge-term vocabulary. `HostBoundary` is the one deliberate change of
        // shape: a must-equal edge on the pinned slot, never a whole-item blocker.
        let expected = [
            (
                BaselineReason::BodyUnavailable,
                TermKind::UnaryInf,
                Some(InfReason::BodyUnavailable),
            ),
            (BaselineReason::HostBoundary, TermKind::MustEqual, None),
            (
                BaselineReason::ViewAbiBoundary,
                TermKind::UnaryInf,
                Some(InfReason::ViewAbi),
            ),
            (
                BaselineReason::CRepresentation,
                TermKind::UnaryInf,
                Some(InfReason::CAbi),
            ),
            (
                BaselineReason::ForeignCall,
                TermKind::UnaryInf,
                Some(InfReason::ForeignCall),
            ),
            (
                BaselineReason::UnknownCall,
                TermKind::UnaryInf,
                Some(InfReason::UnknownCall),
            ),
            (
                BaselineReason::RawProvenance,
                TermKind::UnaryInf,
                Some(InfReason::RawProvenance),
            ),
            (
                BaselineReason::CustomAllocator,
                TermKind::UnaryInf,
                Some(InfReason::CustomAllocator),
            ),
            (
                BaselineReason::UnknownAllocator,
                TermKind::UnaryInf,
                Some(InfReason::UnknownAllocator),
            ),
            (
                BaselineReason::StaticEscape,
                TermKind::UnaryInf,
                Some(InfReason::StaticEscape),
            ),
            (
                BaselineReason::UserDefinedDrop,
                TermKind::UnaryInf,
                Some(InfReason::UserDefinedDrop),
            ),
            (
                BaselineReason::UnsupportedOperation,
                TermKind::UnaryInf,
                Some(InfReason::UnsupportedOperation),
            ),
            (
                BaselineReason::Unsized,
                TermKind::UnaryInf,
                Some(InfReason::Unsized),
            ),
            (
                BaselineReason::ZeroSized,
                TermKind::UnaryInf,
                Some(InfReason::ZeroSized),
            ),
            (
                BaselineReason::UnknownLayout,
                TermKind::UnaryInf,
                Some(InfReason::UnknownLayout),
            ),
            (
                BaselineReason::MixedRepresentations,
                TermKind::UnaryInf,
                Some(InfReason::MixedRepresentations),
            ),
        ];
        for (reason, kind, mapped) in expected {
            assert_eq!(v1_term(reason), (kind, mapped), "{reason:?}");
        }
    }

    #[test]
    fn legality_body_unavailable_is_unary_on_the_opaque_slot_not_the_caller() {
        let mut prog = Prog::new();
        prog.add(
            "caller",
            call_func(1, STRING, direct(2), CallSemantic::Ordinary, None),
        );
        let graph = prog.collect();
        let terms = terms(&graph);
        let opaque = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Opaque { .. }))
            .expect("an opaque callee slot");
        assert!(has_unary(&terms, opaque, InfReason::BodyUnavailable));
        // The caller's argument keeps only its adapter edge: it can stay native and convert.
        let arg = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { .. }))
            .expect("the caller argument");
        for term in terms_on(&terms, arg) {
            assert!(matches!(term, Term::Adapter { .. }), "{term:?}");
        }
    }

    #[test]
    fn legality_host_boundary_pins_the_edge_not_the_item() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(vec![MUT_STRING, STRING], UNIT));
        let pinned = b.param(MUT_STRING, NO_ORIGIN);
        let sibling = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(9),
                function: None,
                args: vec![pinned],
                track_caller: None,
                semantic: CallSemantic::Host,
                host: Some(HostPayload::Map { marker: StrId(0) }),
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let terms = terms(&graph);
        let pinned_key = value_node("f", Site::Value(pinned), &[Step::Deref]);
        assert!(terms.iter().any(|term| matches!(
            term,
            Term::MustEqual { a, b, reason: EqualReason::HostPin }
                if *a == pinned_key && *b == NodeKey::Baseline
        )));
        // The sibling value in the same item carries no term at all.
        let sibling_key = value_node("f", Site::Value(sibling), &[]);
        assert!(terms_on(&terms, &sibling_key).is_empty());
    }

    #[test]
    fn legality_a_v3_reads_slot_is_a_sink_edge_and_retains_mutates_pins() {
        let mut prog = Prog::new();
        let descriptor = Descriptor {
            shape: Shape::Method,
            nullable: false,
            fallible: false,
            global: false,
            module: String::new(),
            name: "write".to_owned(),
            slots: vec![
                (
                    Slot::Parameter(0),
                    SlotFacts::default()
                        .with(Fact::Reads)
                        .with(Fact::HostString),
                ),
                (
                    Slot::Parameter(1),
                    SlotFacts::default().with(Fact::Retains).with(Fact::Mutates),
                ),
            ],
        };
        let encoded = descriptor.encode();
        let body = encoded.strip_prefix(PREFIX).unwrap().to_owned();
        let bytes = prog.interns.intern_bytes(body.as_bytes());
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING, STRING], UNIT));
        let read = b.param(STRING, NO_ORIGIN);
        let kept = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(9),
                function: None,
                args: vec![read, kept],
                track_caller: None,
                semantic: CallSemantic::Host,
                host: Some(HostPayload::Extern(bytes)),
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let read_key = value_node("f", Site::Value(read), &[]);
        let kept_key = value_node("f", Site::Value(kept), &[]);
        assert!(graph.edges.contains(&Edge {
            a: read_key.clone(),
            b: NodeKey::Baseline,
            kind: EdgeKind::HostSink {
                accepts: SlotFacts::default()
                    .with(Fact::Reads)
                    .with(Fact::HostString),
            },
        }));
        assert!(graph.edges.contains(&Edge {
            a: kept_key.clone(),
            b: NodeKey::Baseline,
            kind: EdgeKind::HostPin,
        }));
        // The sink slot's value stays committable, fixed to the one carrier the host accepts;
        // the pinned slot produces exactly the must-equal.
        let terms = terms(&graph);
        assert_eq!(
            terms_on(&terms, &read_key),
            vec![Term::FixedCarrier {
                node: read_key.clone(),
                carrier: Carrier::HostString,
            }]
        );
        assert!(terms.iter().any(|term| matches!(
            term,
            Term::MustEqual { a, reason: EqualReason::HostPin, .. } if *a == kept_key
        )));
    }

    /// The accepts-set is load bearing: a mutable string can only be a builder, which no
    /// declaration can accept, so a `reads, accepts(host_string)` slot receiving a mutated
    /// `String` pins it — `{ text, cap, len }` must never reach a host that declared a JS string.
    #[test]
    fn legality_a_mutated_string_into_an_accepts_host_string_slot_pins() {
        let mut prog = Prog::new();
        let descriptor = Descriptor {
            shape: Shape::Method,
            nullable: false,
            fallible: false,
            global: false,
            module: String::new(),
            name: "write".to_owned(),
            slots: vec![(
                Slot::Parameter(0),
                SlotFacts::default()
                    .with(Fact::Reads)
                    .with(Fact::HostString),
            )],
        };
        let encoded = descriptor.encode();
        let body = encoded.strip_prefix(PREFIX).unwrap().to_owned();
        let bytes = prog.interns.intern_bytes(body.as_bytes());
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let read = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        // A mutable borrow marks the value mutable.
        let borrow = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(read),
                path: Vec::new(),
                mutable: true,
            },
            MUT_STRING,
            NO_ORIGIN,
        );
        let _ = borrow;
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(9),
                function: None,
                args: vec![read],
                track_caller: None,
                semantic: CallSemantic::Host,
                host: Some(HostPayload::Extern(bytes)),
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let read_key = value_node("f", Site::Value(read), &[]);
        assert!(
            graph.nodes[&read_key].mutable
                || graph.nodes.iter().any(|(key, facts)| facts.mutable
                    && matches!(key, NodeKey::Value { item, .. } if item.as_str() == "f")),
            "{}",
            graph.dump()
        );
        let terms = terms(&graph);
        assert!(
            terms.iter().any(|term| matches!(
                term,
                Term::MustEqual { a, reason: EqualReason::HostPin, .. } if *a == read_key
            )),
            "{terms:?}"
        );
        assert!(!terms
            .iter()
            .any(|term| matches!(term, Term::FixedCarrier { node, .. } if *node == read_key)));
    }

    #[test]
    fn legality_view_abi_boundary_is_unary() {
        let mut prog = Prog::new();
        prog.set_type(STRING, |descriptor| {
            descriptor.meta.boundary = PinnedBoundary::ViewAbi;
        });
        prog.add("f", identity_func(1, STRING, STRING));
        let graph = prog.collect();
        let terms = terms(&graph);
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { .. }))
            .unwrap();
        assert!(has_unary(&terms, key, InfReason::ViewAbi));
    }

    #[test]
    fn legality_c_representation_is_unary() {
        let mut prog = Prog::new();
        prog.set_type(STRING, |descriptor| {
            descriptor.meta.abi = NominalAbi::C;
        });
        prog.add("f", identity_func(1, STRING, STRING));
        let graph = prog.collect();
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { .. }))
            .unwrap();
        assert!(has_unary(&terms(&graph), key, InfReason::CAbi));
    }

    #[test]
    fn legality_foreign_call_is_unary_on_the_foreign_slot() {
        let mut prog = Prog::new();
        prog.add(
            "caller",
            call_func(
                1,
                STRING,
                Callee::Foreign {
                    definition: def(70),
                    symbol: StrId(0),
                },
                CallSemantic::Ordinary,
                None,
            ),
        );
        let graph = prog.collect();
        let terms = terms(&graph);
        let opaque = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Opaque { .. }))
            .expect("the foreign slot");
        assert!(has_unary(&terms, opaque, InfReason::ForeignCall));
    }

    #[test]
    fn legality_unknown_call_is_unary_where_dispatch_cannot_be_grouped() {
        let mut prog = Prog::new();
        // An indirect call at a signature that is not a fn-pointer type in the table.
        prog.add(
            "caller",
            call_func(
                1,
                STRING,
                Callee::Indirect {
                    signature: TypeId(999),
                },
                CallSemantic::Ordinary,
                None,
            ),
        );
        let graph = prog.collect();
        let terms = terms(&graph);
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { .. }))
            .unwrap();
        assert!(has_unary(&terms, key, InfReason::UnknownCall));
    }

    #[test]
    fn legality_raw_provenance_is_unary() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.op(
            entry,
            OpKind::RawView {
                root: Root::Value(p),
                path: Vec::new(),
                mutable: false,
            },
            REF_STRING,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let key = value_node("f", Site::Value(p), &[]);
        assert!(has_unary(&terms(&graph), &key, InfReason::RawProvenance));
    }

    #[test]
    fn legality_custom_allocator_is_unary() {
        let mut prog = Prog::new();
        prog.set_type(STRING, |descriptor| {
            descriptor.meta.allocator = AllocatorIdentity::Custom;
        });
        prog.add("f", identity_func(1, STRING, STRING));
        let graph = prog.collect();
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { .. }))
            .unwrap();
        assert!(has_unary(&terms(&graph), key, InfReason::CustomAllocator));
    }

    #[test]
    fn legality_an_unproven_artifact_allocator_is_unary_on_every_value() {
        let mut prog = Prog::new();
        prog.add("f", identity_func(1, STRING, STRING));
        prog.unknown_allocator();
        let graph = prog.collect();
        let terms = terms(&graph);
        for key in graph.nodes.keys() {
            if matches!(key, NodeKey::Value { .. }) {
                assert!(has_unary(&terms, key, InfReason::UnknownAllocator), "{key}");
            }
        }
    }

    #[test]
    fn legality_static_escape_is_unary_on_the_static_node() {
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        b.op(
            entry,
            OpKind::Const(ConstTree::Global {
                target: GlobalReference::Static {
                    definition: def(60),
                    link_name: StrId(0),
                },
                offset: 0,
            }),
            REF_STRING,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let graph = prog.collect();
        let terms = terms(&graph);
        let static_node = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Static { .. }))
            .expect("the static node");
        assert!(has_unary(&terms, static_node, InfReason::StaticEscape));
    }

    #[test]
    fn legality_user_defined_drop_is_unary() {
        let mut prog = Prog::new();
        prog.add("f", identity_func(1, DROPPY, DROPPY));
        let graph = prog.collect();
        let terms = terms(&graph);
        // The `String` inside the struct with its own destructor inherits the reason.
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { path, .. } if path == &[Step::Field(0)]))
            .expect("the field node");
        assert!(has_unary(&terms, key, InfReason::UserDefinedDrop));
    }

    #[test]
    fn legality_unsupported_operation_is_unary() {
        let mut prog = Prog::new();
        prog.add_semantic(
            "swap",
            identity_func(1, MUT_VEC, UNIT),
            BodySemantic::Standard(StandardOperation::VecSwap),
        );
        let graph = prog.collect();
        let terms = terms(&graph);
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { .. }))
            .unwrap();
        assert!(has_unary(&terms, key, InfReason::UnsupportedOperation));
    }

    /// The shared-receiver builder-only class (`StringCapacity`, the D2 cut): the requirement's
    /// carrier set narrows the caller's own node to a [`Term::FixedCarrier`] — a host-string
    /// label could not answer `.cap`.
    #[test]
    fn legality_a_builder_only_cut_slot_fixes_the_caller_node() {
        use crate::jir::repr::{FuncBuilder, OpKind, RTerm, Root};
        let mut prog = Prog::new();
        prog.add("capacity_like", identity_func(2, REF_STRING, UNIT));
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let r = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(p),
                path: Vec::new(),
                mutable: false,
            },
            REF_STRING,
            NO_ORIGIN,
        );
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![r],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::StringCapacity),
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("caller", b.finish());
        let graph = prog.collect();
        let terms = terms(&graph);
        let key = value_node("caller", Site::Value(r), &[Step::Deref]);
        assert!(
            terms.iter().any(|term| matches!(
                term,
                Term::FixedCarrier { node, carrier: Carrier::StringBuilder } if *node == key
            )),
            "{terms:?}"
        );
    }

    #[test]
    fn legality_an_unsized_vec_element_is_unary() {
        let mut prog = Prog::new();
        prog.set_type(STRING, |descriptor| {
            descriptor.layout.sized = false;
            descriptor.layout.size = None;
        });
        prog.add("f", identity_func(1, VEC, VEC));
        let graph = prog.collect();
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { path, .. } if path.is_empty()))
            .unwrap();
        assert!(has_unary(&terms(&graph), key, InfReason::Unsized));
    }

    #[test]
    fn legality_a_zero_sized_vec_element_is_unary() {
        let mut prog = Prog::new();
        prog.set_type(STRING, |descriptor| {
            descriptor.layout.zst = true;
            descriptor.layout.size = Some(0);
        });
        prog.add("f", identity_func(1, VEC, VEC));
        let graph = prog.collect();
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { path, .. } if path.is_empty()))
            .unwrap();
        assert!(has_unary(&terms(&graph), key, InfReason::ZeroSized));
    }

    #[test]
    fn legality_an_unknown_element_layout_is_unary() {
        let mut prog = Prog::new();
        prog.set_type(STRING, |descriptor| {
            descriptor.layout.size = None;
            descriptor.layout.align = None;
        });
        prog.add("f", identity_func(1, VEC, VEC));
        let graph = prog.collect();
        let key = graph
            .nodes
            .keys()
            .find(|key| matches!(key, NodeKey::Value { path, .. } if path.is_empty()))
            .unwrap();
        assert!(has_unary(&terms(&graph), key, InfReason::UnknownLayout));
    }

    #[test]
    fn legality_mixed_candidate_kinds_in_one_union_are_unary_on_the_representative() {
        let mut graph = ValueGraph::default();
        let a = value_node("f", Site::Value(crate::jir::repr::ValueId(0)), &[]);
        let b = value_node("f", Site::Value(crate::jir::repr::ValueId(1)), &[]);
        graph.nodes.insert(
            a.clone(),
            NodeFacts {
                kind: Some(CandidateKind::String),
                ..NodeFacts::default()
            },
        );
        graph.nodes.insert(
            b.clone(),
            NodeFacts {
                kind: Some(CandidateKind::Vec),
                ..NodeFacts::default()
            },
        );
        graph.unions.insert((a.clone(), b));
        let terms = terms(&graph);
        assert!(has_unary(&terms, &a, InfReason::MixedRepresentations));
    }

    #[test]
    fn legality_adapter_terms_carry_the_dead_source_direction() {
        let mut graph = ValueGraph::default();
        let a = value_node("f", Site::Value(crate::jir::repr::ValueId(0)), &[]);
        let b = value_node("g", Site::Value(crate::jir::repr::ValueId(0)), &[]);
        graph.nodes.insert(a.clone(), NodeFacts::default());
        graph.nodes.insert(b.clone(), NodeFacts::default());
        graph.edges.insert(Edge {
            a: a.clone(),
            b: b.clone(),
            kind: EdgeKind::Value { source_dead: true },
        });
        let with_dead = terms(&graph);
        assert!(with_dead.iter().any(|term| matches!(
            term,
            Term::Adapter {
                baseline_to_native_legal: true,
                ..
            }
        )));
        graph.edges.clear();
        graph.edges.insert(Edge {
            a,
            b,
            kind: EdgeKind::Value { source_dead: false },
        });
        let without = terms(&graph);
        assert!(without.iter().any(|term| matches!(
            term,
            Term::Adapter {
                baseline_to_native_legal: false,
                ..
            }
        )));
    }

    #[test]
    fn legality_export_round_trips_are_must_equal() {
        let mut prog = Prog::new();
        prog.add("take", identity_func(1, STRING, UNIT));
        prog.add("give", identity_func(2, STRING, STRING));
        prog.exported("take");
        prog.exported("give");
        let graph = prog.collect();
        let terms = terms(&graph);
        assert!(terms.iter().any(|term| matches!(
            term,
            Term::MustEqual {
                reason: EqualReason::ExportRoundTrip,
                ..
            }
        )));
    }

    #[test]
    fn legality_terms_never_name_a_component() {
        // Structural guard for the review checklist: the vocabulary has no region-shaped term.
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        prog.add(
            "caller",
            call_func(1, STRING, direct(2), CallSemantic::Ordinary, None),
        );
        let graph = prog.collect();
        for term in terms(&graph) {
            match term {
                Term::UnaryInf { .. }
                | Term::MustEqual { .. }
                | Term::Adapter { .. }
                | Term::FixedCarrier { .. } => {}
            }
        }
    }
}
