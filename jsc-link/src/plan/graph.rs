//! Problem construction for the representation solver: the value graph and its legality terms
//! flattened into the node-and-arc shape the min-cut families run over (D3).
//!
//! Nothing here solves. [`Problem::build`] collapses each item's intra-value unions into one
//! node, attaches the unary terms, turns every edge term into a directed arc pair, and adds the
//! facility nodes: one [`ProblemKey::Version`] per recursive call SCC that owns candidate nodes
//! (the cost of cloning a `$n` native version, tied to its users by INF implication arcs), and a
//! [`ProblemKey::Helper`] per shim member a labeling needs (added by the registry's consumer
//! through [`Problem::add_helper`]). The solver reads capacities by class: an [`Cap::Inf`] arc is
//! the legality boundary, a [`Cap::Finite`] arc is priced by the cost model, and [`Cap::Free`]
//! costs nothing.
//!
//! Arc direction follows the cut convention of `research/v2/representation_mincut.py`: source
//! side is native, sink side is baseline, and the capacity of `a -> b` is paid when `a` labels
//! native beside `b` baseline. A must-equal term is INF both ways; an adapter term is finite in
//! the native-to-baseline direction and finite the other way only where the source was proven
//! dead and unaliased; an implication `user native => facility native` is INF from user to
//! facility and free back.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::analysis::legality::{InfReason, Term};
use crate::analysis::valueflow::{CandidateKind, NodeKey, UnionFind, ValueGraph};
use crate::item::JsName;
use crate::jir::repr::Carrier;

/// A capacity class; the cost model replaces `Finite` with measured bytes (W4e/W4b).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Cap {
    Free,
    Finite,
    Inf,
}

impl Cap {
    fn max(self, other: Cap) -> Cap {
        std::cmp::max(self, other)
    }
}

/// What one problem node stands for.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProblemKey {
    /// The representative of one collapsed value-graph node.
    Value(NodeKey),
    /// The baseline anchor: always on the sink side.
    Baseline,
    /// The `$n` native version of one recursive call SCC: native exactly when any of its users
    /// is, at the version's clone cost.
    Version { scc: u32, items: Vec<JsName> },
    /// A `__rt.*` shim member some native labeling needs, at its member-source byte cost.
    Helper(String),
    /// An item that is reachable only through the baseline bodies of candidate operations: it
    /// drops out of the program when all of its users commit, which is the native-side saving
    /// (the "complete reachable item graph" of v1's region cost, as a cut node).
    Dependency(JsName),
}

impl fmt::Display for ProblemKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(key) => write!(f, "{key}"),
            Self::Baseline => write!(f, "baseline"),
            Self::Version { scc, items } => {
                write!(f, "version:{scc}[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(",")?;
                    }
                    f.write_str(item.as_str())?;
                }
                write!(f, "]")
            }
            Self::Helper(member) => write!(f, "helper:{member}"),
            Self::Dependency(item) => write!(f, "dep:{}", item.as_str()),
        }
    }
}

/// One node of the solver's problem.
#[derive(Clone, Debug)]
pub struct ProblemNode {
    pub key: ProblemKey,
    /// The candidate family, for the F2/F3 sub-solves.
    pub family: Option<CandidateKind>,
    /// Whether anything writes the value: what separates HostString from StringBuilder.
    pub mutable: bool,
    /// The one carrier a native label may take, from a [`Term::FixedCarrier`] (a host sink slot
    /// whose declaration accepts exactly one solvable carrier). `None` leaves the family
    /// sub-solve free.
    pub fixed: Option<Carrier>,
    /// The unary reasons; any entry makes the node's native label infinite.
    pub inf: BTreeSet<InfReason>,
    /// How many value-graph nodes collapsed into this one.
    pub members: u32,
}

/// One directed arc pair between problem nodes: `ab` is paid when `a` is native beside `b`
/// baseline, `ba` the other way around.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Arc {
    pub a: usize,
    pub b: usize,
    pub ab: Cap,
    pub ba: Cap,
}

/// The flattened labeling problem.
#[derive(Clone, Debug, Default)]
pub struct Problem {
    pub nodes: Vec<ProblemNode>,
    pub arcs: Vec<Arc>,
    /// The index of [`ProblemKey::Baseline`].
    pub baseline: usize,
    /// Every graph node to the problem node its union collapsed into: how the solver's commit
    /// maps a concrete `(item, value, path)` back to its label (W4b).
    pub resolution: BTreeMap<NodeKey, usize>,
}

impl Problem {
    /// Flattens `graph` and `terms` into the solver's shape. Deterministic: nodes are ordered by
    /// their representative key, arcs by endpoint indices.
    pub fn build(graph: &ValueGraph, terms: &[Term]) -> Problem {
        let keys: Vec<&NodeKey> = graph.nodes.keys().collect();
        let index: BTreeMap<&NodeKey, usize> =
            keys.iter().enumerate().map(|(i, key)| (*key, i)).collect();
        let mut unions = UnionFind::new(keys.len());
        for (a, b) in &graph.unions {
            if let (Some(a), Some(b)) = (index.get(a), index.get(b)) {
                unions.union(*a, *b);
            }
        }
        // Dense problem indices for the representatives, in key order.
        let mut dense: BTreeMap<usize, usize> = BTreeMap::new();
        let mut nodes = Vec::new();
        let mut baseline = 0;
        // The representative of a union is its smallest index, so it is always visited first.
        #[allow(clippy::needless_range_loop)] // `unions.root(i)` needs the index itself.
        for i in 0..keys.len() {
            let root = unions.root(i);
            let facts = &graph.nodes[keys[i]];
            if root != i {
                let node = &mut nodes[dense[&root]];
                merge(node, facts);
                node.members += 1;
                continue;
            }
            let problem_index = nodes.len();
            dense.insert(root, problem_index);
            let key = if *keys[i] == NodeKey::Baseline {
                baseline = problem_index;
                ProblemKey::Baseline
            } else {
                ProblemKey::Value(keys[i].clone())
            };
            let mut node = ProblemNode {
                key,
                family: None,
                mutable: false,
                fixed: None,
                inf: BTreeSet::new(),
                members: 1,
            };
            merge(&mut node, facts);
            nodes.push(node);
        }
        let mut resolve = |key: &NodeKey| index.get(key).map(|i| dense[&unions.root(*i)]);

        // Terms onto nodes and arcs.
        let mut arcs: BTreeMap<(usize, usize), (Cap, Cap)> = BTreeMap::new();
        let mut arc = |a: usize, b: usize, ab: Cap, ba: Cap| {
            if a == b {
                return;
            }
            let (x, y, ab, ba) = if a < b {
                (a, b, ab, ba)
            } else {
                (b, a, ba, ab)
            };
            let entry = arcs.entry((x, y)).or_insert((Cap::Free, Cap::Free));
            entry.0 = entry.0.max(ab);
            entry.1 = entry.1.max(ba);
        };
        let mut unary: Vec<(usize, InfReason)> = Vec::new();
        let mut fixed: Vec<(usize, Carrier)> = Vec::new();
        for term in terms {
            match term {
                Term::UnaryInf { node, reason } => {
                    if let Some(node) = resolve(node) {
                        unary.push((node, *reason));
                    }
                }
                Term::FixedCarrier { node, carrier } => {
                    if let Some(node) = resolve(node) {
                        fixed.push((node, *carrier));
                    }
                }
                Term::MustEqual { a, b, .. } => {
                    if let (Some(a), Some(b)) = (resolve(a), resolve(b)) {
                        arc(a, b, Cap::Inf, Cap::Inf);
                    }
                }
                Term::Adapter {
                    a,
                    b,
                    baseline_to_native_legal,
                } => {
                    if let (Some(a), Some(b)) = (resolve(a), resolve(b)) {
                        let ba = if *baseline_to_native_legal {
                            Cap::Finite
                        } else {
                            Cap::Inf
                        };
                        arc(a, b, Cap::Finite, ba);
                    }
                }
            }
        }
        for (node, reason) in unary {
            nodes[node].inf.insert(reason);
        }
        // A collapsed node fixed to two different carriers: only the baseline satisfies both
        // declarations, so the node pins exactly as a must-equal against the anchor would.
        let mut conflicted: BTreeSet<usize> = BTreeSet::new();
        for (node, carrier) in fixed {
            match nodes[node].fixed {
                Some(existing) if existing != carrier => {
                    conflicted.insert(node);
                }
                _ => nodes[node].fixed = Some(carrier),
            }
        }
        for node in conflicted {
            nodes[node].fixed = None;
            arc(node, baseline, Cap::Inf, Cap::Inf);
        }

        let resolution = index
            .iter()
            .map(|(key, i)| ((*key).clone(), dense[&unions.root(*i)]))
            .collect();
        let mut problem = Problem {
            nodes,
            arcs: arcs
                .into_iter()
                .map(|((a, b), (ab, ba))| Arc { a, b, ab, ba })
                .collect(),
            baseline,
            resolution,
        };
        problem.versions(graph, &dense, &mut unions, &index);
        problem
    }

    /// One `Version` facility node per recursive call SCC that owns candidate nodes, tied to
    /// every one of its value nodes by an implication arc.
    fn versions(
        &mut self,
        graph: &ValueGraph,
        dense: &BTreeMap<usize, usize>,
        unions: &mut UnionFind,
        index: &BTreeMap<&NodeKey, usize>,
    ) {
        // The items owning at least one problem node, and the problem nodes of each.
        let mut owners: BTreeMap<JsName, Vec<usize>> = BTreeMap::new();
        for (key, i) in index {
            if let NodeKey::Value { item, .. } = key {
                owners
                    .entry(item.clone())
                    .or_default()
                    .push(dense[&unions.root(*i)]);
            }
        }
        for members in owners.values_mut() {
            members.sort_unstable();
            members.dedup();
        }
        let mut scc_index = 0;
        for component in sccs(&graph.direct_calls) {
            let items: Vec<JsName> = component
                .into_iter()
                .filter(|item| owners.contains_key(item))
                .collect();
            if items.is_empty() {
                continue;
            }
            let facility = self.nodes.len();
            self.nodes.push(ProblemNode {
                key: ProblemKey::Version {
                    scc: scc_index,
                    items: items.clone(),
                },
                family: None,
                mutable: false,
                fixed: None,
                inf: BTreeSet::new(),
                members: 0,
            });
            for item in &items {
                for user in &owners[item] {
                    self.arcs.push(Arc {
                        a: *user,
                        b: facility,
                        ab: Cap::Inf,
                        ba: Cap::Free,
                    });
                }
            }
            scc_index += 1;
        }
    }

    /// Adds a `Helper` facility node needed by `users` and returns its index.
    pub fn add_helper(&mut self, member: &str, users: &[usize]) -> usize {
        let facility = self.nodes.len();
        self.nodes.push(ProblemNode {
            key: ProblemKey::Helper(member.to_owned()),
            family: None,
            mutable: false,
            fixed: None,
            inf: BTreeSet::new(),
            members: 0,
        });
        for user in users {
            self.arcs.push(Arc {
                a: *user,
                b: facility,
                ab: Cap::Inf,
                ba: Cap::Free,
            });
        }
        facility
    }

    /// A deterministic text dump: the fixture and review format.
    #[allow(dead_code)] // The unit tests and reviews read it; builds do not.
    pub fn dump(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let _ = write!(out, "n{i} {}", node.key);
            if let Some(family) = node.family {
                let _ = write!(out, " {family:?}");
            }
            if node.mutable {
                out.push_str(" mut");
            }
            for reason in &node.inf {
                let _ = write!(out, " !{}", reason.as_str());
            }
            let _ = writeln!(out, " x{}", node.members);
        }
        for arc in &self.arcs {
            let cap = |cap: Cap| match cap {
                Cap::Free => "0",
                Cap::Finite => "$",
                Cap::Inf => "inf",
            };
            let _ = writeln!(
                out,
                "arc n{} <{}/{}> n{}",
                arc.a,
                cap(arc.ab),
                cap(arc.ba),
                arc.b
            );
        }
        out
    }
}

fn merge(node: &mut ProblemNode, facts: &crate::analysis::valueflow::NodeFacts) {
    node.family = node.family.or(facts.kind);
    node.mutable |= facts.mutable;
    node.inf.extend(facts.reasons.iter().copied());
}

/// The strongly connected components of the direct-call graph, each sorted, in deterministic
/// order (v1 `direct_call_sccs`).
fn sccs(calls: &BTreeMap<JsName, BTreeSet<JsName>>) -> Vec<Vec<JsName>> {
    let mut vertices: BTreeSet<&JsName> = BTreeSet::new();
    for (caller, callees) in calls {
        vertices.insert(caller);
        vertices.extend(callees.iter());
    }
    let graph: BTreeMap<&JsName, Vec<&JsName>> = vertices
        .iter()
        .map(|vertex| {
            let edges = calls
                .get(*vertex)
                .into_iter()
                .flatten()
                .filter(|callee| vertices.contains(callee))
                .collect();
            (*vertex, edges)
        })
        .collect();
    let mut reverse: BTreeMap<&JsName, Vec<&JsName>> = vertices
        .iter()
        .map(|vertex| (*vertex, Vec::new()))
        .collect();
    for (caller, callees) in &graph {
        for callee in callees {
            reverse
                .get_mut(*callee)
                .expect("every callee is a vertex")
                .push(*caller);
        }
    }

    let mut seen: BTreeSet<&JsName> = BTreeSet::new();
    let mut finished: Vec<&JsName> = Vec::new();
    for vertex in &vertices {
        if seen.contains(*vertex) {
            continue;
        }
        let mut stack = vec![(*vertex, false)];
        while let Some((vertex, exiting)) = stack.pop() {
            if exiting {
                finished.push(vertex);
                continue;
            }
            if !seen.insert(vertex) {
                continue;
            }
            stack.push((vertex, true));
            for next in &graph[vertex] {
                if !seen.contains(*next) {
                    stack.push((*next, false));
                }
            }
        }
    }

    seen.clear();
    let mut components = Vec::new();
    for vertex in finished.into_iter().rev() {
        if seen.contains(vertex) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![vertex];
        while let Some(vertex) = stack.pop() {
            if !seen.insert(vertex) {
                continue;
            }
            component.push(vertex.clone());
            for next in &reverse[vertex] {
                if !seen.contains(*next) {
                    stack.push(*next);
                }
            }
        }
        component.sort();
        components.push(component);
    }
    components.sort();
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::legality::{self, InfReason};
    use crate::analysis::valueflow::harness::*;
    use crate::analysis::valueflow::{Edge, EdgeKind, NodeFacts, NodeKey, Site, ValueGraph};
    use crate::jir::repr::ValueId;
    use crate::jir::semantic::CallSemantic;

    fn node(item: &str, value: u32) -> NodeKey {
        value_node(item, Site::Value(ValueId(value)), &[])
    }

    fn graph_with(nodes: &[NodeKey]) -> ValueGraph {
        let mut graph = ValueGraph::default();
        graph.nodes.insert(NodeKey::Baseline, NodeFacts::default());
        for key in nodes {
            graph.nodes.insert(key.clone(), NodeFacts::default());
        }
        graph
    }

    fn index_of(problem: &Problem, key: &NodeKey) -> usize {
        problem
            .nodes
            .iter()
            .position(|node| matches!(&node.key, ProblemKey::Value(found) if found == key))
            .expect("the node is in the problem")
    }

    #[test]
    fn unions_collapse_into_one_node_and_must_equal_is_inf_both_ways() {
        let a = node("f", 0);
        let b = node("f", 1);
        let c = node("g", 0);
        let mut graph = graph_with(&[a.clone(), b.clone(), c.clone()]);
        graph.unions.insert((a.clone(), b.clone()));
        graph.edges.insert(Edge {
            a: b.clone(),
            b: c.clone(),
            kind: EdgeKind::Shared,
        });
        let terms = legality::terms(&graph);
        let problem = Problem::build(&graph, &terms);
        // Baseline + the collapsed pair + c.
        assert_eq!(problem.nodes.len(), 3);
        let ab = index_of(&problem, &a);
        assert_eq!(problem.nodes[ab].members, 2);
        let c_index = index_of(&problem, &c);
        let arc = problem
            .arcs
            .iter()
            .find(|arc| (arc.a == ab && arc.b == c_index) || (arc.a == c_index && arc.b == ab))
            .expect("the must-equal arc");
        assert_eq!((arc.ab, arc.ba), (Cap::Inf, Cap::Inf));
    }

    #[test]
    fn adapter_arcs_are_asymmetric_in_the_dead_source_direction() {
        let a = node("f", 0);
        let b = node("g", 0);
        let mut graph = graph_with(&[a.clone(), b.clone()]);
        graph.edges.insert(Edge {
            a: a.clone(),
            b: b.clone(),
            kind: EdgeKind::Value { source_dead: false },
        });
        let terms = legality::terms(&graph);
        let problem = Problem::build(&graph, &terms);
        let ai = index_of(&problem, &a);
        let bi = index_of(&problem, &b);
        let arc = problem.arcs.iter().find(|arc| arc.a != arc.b).unwrap();
        // `a` native beside `b` baseline is the materialize direction: finite. The reverse needs
        // the dead source and is infinite here.
        let (ab, ba) = if (arc.a, arc.b) == (ai, bi) {
            (arc.ab, arc.ba)
        } else {
            assert_eq!((arc.a, arc.b), (bi, ai));
            (arc.ba, arc.ab)
        };
        assert_eq!((ab, ba), (Cap::Finite, Cap::Inf));
    }

    #[test]
    fn unary_terms_land_on_the_collapsed_node() {
        let a = node("f", 0);
        let b = node("f", 1);
        let mut graph = graph_with(&[a.clone(), b.clone()]);
        graph.unions.insert((a.clone(), b.clone()));
        graph
            .nodes
            .get_mut(&b)
            .unwrap()
            .reasons
            .insert(InfReason::RawProvenance);
        let terms = legality::terms(&graph);
        let problem = Problem::build(&graph, &terms);
        let ab = index_of(&problem, &a);
        assert!(problem.nodes[ab].inf.contains(&InfReason::RawProvenance));
    }

    /// A fixed-carrier term lands on the collapsed node; two declarations fixing one node to two
    /// different carriers pin it to the baseline anchor instead (only the baseline satisfies
    /// both).
    #[test]
    fn fixed_carrier_terms_land_on_the_node_and_conflicts_pin_to_baseline() {
        use crate::analysis::legality::Term;
        use crate::jir::repr::Carrier;
        let a = node("f", 0);
        let b = node("g", 0);
        let graph = graph_with(&[a.clone(), b.clone()]);
        let terms = vec![
            Term::FixedCarrier {
                node: a.clone(),
                carrier: Carrier::HostString,
            },
            Term::FixedCarrier {
                node: b.clone(),
                carrier: Carrier::HostString,
            },
            Term::FixedCarrier {
                node: b.clone(),
                carrier: Carrier::NativeVec,
            },
        ];
        let problem = Problem::build(&graph, &terms);
        let ai = index_of(&problem, &a);
        let bi = index_of(&problem, &b);
        assert_eq!(problem.nodes[ai].fixed, Some(Carrier::HostString));
        assert_eq!(problem.nodes[bi].fixed, None);
        let arc = problem
            .arcs
            .iter()
            .find(|arc| {
                (arc.a == bi && arc.b == problem.baseline)
                    || (arc.a == problem.baseline && arc.b == bi)
            })
            .expect("the conflict pins to baseline");
        assert_eq!((arc.ab, arc.ba), (Cap::Inf, Cap::Inf));
        assert!(
            !problem.arcs.iter().any(|arc| arc.a == ai || arc.b == ai),
            "a clean fix adds no arc"
        );
    }

    #[test]
    fn a_recursive_scc_with_candidate_nodes_gets_one_version_facility() {
        let mut prog = Prog::new();
        // `f` and `g` call each other, both carrying a string argument.
        prog.add(
            "f",
            call_func(1, STRING, direct(2), CallSemantic::Ordinary, None),
        );
        prog.add(
            "g",
            call_func(2, STRING, direct(1), CallSemantic::Ordinary, None),
        );
        let graph = prog.collect();
        let terms = legality::terms(&graph);
        let problem = Problem::build(&graph, &terms);
        let versions: Vec<&ProblemNode> = problem
            .nodes
            .iter()
            .filter(|node| matches!(node.key, ProblemKey::Version { .. }))
            .collect();
        assert_eq!(versions.len(), 1, "{}", problem.dump());
        let ProblemKey::Version { items, .. } = &versions[0].key else {
            unreachable!()
        };
        assert_eq!(
            items.iter().map(|item| item.as_str()).collect::<Vec<_>>(),
            ["f", "g"]
        );
        // Implication arcs run user -> facility at INF, free back.
        let facility = problem
            .nodes
            .iter()
            .position(|node| matches!(node.key, ProblemKey::Version { .. }))
            .unwrap();
        let implications: Vec<&Arc> = problem
            .arcs
            .iter()
            .filter(|arc| arc.b == facility)
            .collect();
        assert!(!implications.is_empty());
        for arc in implications {
            assert_eq!((arc.ab, arc.ba), (Cap::Inf, Cap::Free));
        }
    }

    #[test]
    fn a_helper_facility_ties_to_its_users() {
        let a = node("f", 0);
        let graph = graph_with(std::slice::from_ref(&a));
        let terms = legality::terms(&graph);
        let mut problem = Problem::build(&graph, &terms);
        let user = index_of(&problem, &a);
        let helper = problem.add_helper("convert_str", &[user]);
        assert!(matches!(
            &problem.nodes[helper].key,
            ProblemKey::Helper(member) if member == "convert_str"
        ));
        let arc = problem.arcs.iter().find(|arc| arc.b == helper).unwrap();
        assert_eq!((arc.a, arc.ab, arc.ba), (user, Cap::Inf, Cap::Free));
    }

    #[test]
    fn problem_construction_is_deterministic() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        prog.add(
            "caller",
            call_func(1, STRING, direct(2), CallSemantic::Ordinary, None),
        );
        prog.add("give", identity_func(3, VEC, VEC));
        prog.exported("give");
        let build = || {
            let graph = prog.collect();
            let terms = legality::terms(&graph);
            Problem::build(&graph, &terms).dump()
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn sccs_are_deterministic_and_sorted() {
        let mut calls: std::collections::BTreeMap<
            crate::item::JsName,
            std::collections::BTreeSet<crate::item::JsName>,
        > = Default::default();
        let name = |s: &str| crate::item::JsName::new(s.to_owned());
        calls.entry(name("a")).or_default().insert(name("b"));
        calls.entry(name("b")).or_default().insert(name("a"));
        calls.entry(name("b")).or_default().insert(name("c"));
        let components = sccs(&calls);
        assert_eq!(
            components,
            vec![vec![name("a"), name("b")], vec![name("c")]]
        );
    }
}
