//! The lane-time call graph over phase A's [`Func`]s, and its strongly connected components.
//!
//! Nodes are the offered functions phase A built (`link/lane.rs`), identified by [`JsName`]:
//! the link name is the one identity a body has exactly one of ([`ItemId`] is NOT unique --
//! rustc emits `FnOnce::call_once` both as the instance and as its by-value shim, two bodies
//! under one item), and a direct call names its target by `link_name`. An edge points from a
//! caller to everything whose *body* the caller's effects depend on:
//!
//! * the callee of a plain direct call ([`Callee::Direct`]), except a [`CallSemantic::Standard`]
//!   call the semantic-operation registry covers, whose contract
//!   ([`crate::lower::semantic_ops::SemanticOp::effects`]) stands in for the body;
//! * the drop glue a [`OpKind::Drop`] names directly;
//! * every reified target of a fn-pointer signature the caller dispatches through
//!   ([`Callee::Indirect`]), where the program-wide reference scan ([`refs::scan_op`]) bounds that
//!   signature's target set. An unbounded signature contributes no edge; the effects transfer
//!   treats such a call as unknowable instead.
//!
//! The components come out of Tarjan's algorithm in reverse topological order (callees before
//! callers), which is the order a bottom-up interprocedural fixpoint wants
//! ([`crate::analysis::effects`]) and the order the inliner visits (phase 5, D6). Everything is
//! deterministic: nodes and edge sets are `BTreeMap`/`BTreeSet`, members of one component are
//! sorted by `ItemId`.

// The effects summaries and their wave 2 consumers (inline, devirt) are the readers; until the
// O group lands only the summary computation and the tests call this.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::refs::{self, RefScan};
use crate::item::JsName;
use crate::jir::decline::Decline;
use crate::jir::interns::Interns;
use crate::jir::repr::{Func, OpKind};
use crate::jir::semantic::{CallSemantic, Callee, DropGlue, ItemId, TypeId};

/// The direct-call graph of one lane pass, with its condensation.
#[derive(Clone, Debug, Default)]
pub struct Callgraph {
    /// The body-dependency edges of each node: every offered function phase A built, by name.
    /// Targets outside the map's keys (a callee the lane did not build) are kept: the effects
    /// transfer must see the call and find no summary.
    pub callees: BTreeMap<JsName, BTreeSet<JsName>>,
    /// The program-wide reified fn-pointer targets by fn-pointer type: the bound on
    /// [`Callee::Indirect`] dispatch where one exists. Targets are items, as the reference
    /// scan records them; [`Callgraph::by_item`] maps each to its bodies.
    pub fn_pointer_targets: BTreeMap<TypeId, BTreeSet<ItemId>>,
    /// Every name whose `Func` carries the item: more than one where rustc emitted an item and
    /// its shim as separate bodies.
    pub by_item: BTreeMap<ItemId, BTreeSet<JsName>>,
    /// Strongly connected components in reverse topological order (callees before callers),
    /// members sorted by name.
    pub sccs: Vec<Vec<JsName>>,
}

impl Callgraph {
    /// Whether `name` sits on a cycle: in a component with others, or calling itself.
    pub fn recursive(&self, name: &JsName) -> bool {
        self.sccs
            .iter()
            .any(|scc| scc.len() > 1 && scc.contains(name))
            || self
                .callees
                .get(name)
                .is_some_and(|callees| callees.contains(name))
    }
}

/// Builds the graph of every `Ok` func, with `interns` mapping each item to the interns of the
/// object it was read from (the caller builds it from `LinkJir::interns`).
pub fn build(
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &Interns>,
) -> Callgraph {
    // The program-wide fn-pointer target scan, shared with the flat-ABI plan's facts.
    let mut scan = RefScan::default();
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        let table = interns.get(name).map(|interns| &interns.types);
        for block in &func.blocks {
            for op in &block.ops {
                refs::scan_op(func, op, table, &mut scan);
            }
        }
    }

    let mut graph = Callgraph {
        fn_pointer_targets: scan.fn_pointer_targets,
        ..Callgraph::default()
    };
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        graph
            .by_item
            .entry(func.item)
            .or_default()
            .insert(name.clone());
    }
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        let interns = interns.get(name).copied();
        let callees = graph.callees.entry(name.clone()).or_default();
        for block in &func.blocks {
            for op in &block.ops {
                edge_targets(
                    &op.kind,
                    interns,
                    &graph.fn_pointer_targets,
                    &graph.by_item,
                    callees,
                );
            }
        }
    }
    let nodes: Vec<JsName> = graph.callees.keys().cloned().collect();
    graph.sccs = tarjan(&nodes, &graph.callees);
    graph
}

/// Adds the names whose bodies `op` makes the enclosing function's effects depend on. A direct
/// callee resolves by its `link_name` through the caller's interns; an unresolvable one adds
/// no edge, and the effects transfer then finds no summary at the site, which is the same
/// conservative answer.
fn edge_targets(
    op: &OpKind,
    interns: Option<&Interns>,
    fn_pointer_targets: &BTreeMap<TypeId, BTreeSet<ItemId>>,
    by_item: &BTreeMap<ItemId, BTreeSet<JsName>>,
    out: &mut BTreeSet<JsName>,
) {
    let resolve = |link_name: &crate::jir::interns::StrId| {
        interns
            .and_then(|interns| interns.str(*link_name))
            .map(|name| JsName::new(name.to_owned()))
    };
    match op {
        OpKind::Call {
            callee, semantic, ..
        } => match callee {
            Callee::Direct { link_name, .. } => {
                let covered = matches!(
                    semantic,
                    CallSemantic::Standard(operation)
                        if crate::lower::semantic_ops::registry(*operation).is_some()
                );
                if !covered {
                    out.extend(resolve(link_name));
                }
            }
            Callee::Indirect { signature } => {
                if let Some(targets) = fn_pointer_targets.get(signature) {
                    for target in targets {
                        if let Some(names) = by_item.get(target) {
                            out.extend(names.iter().cloned());
                        }
                    }
                }
            }
            _ => {}
        },
        OpKind::Drop {
            glue: DropGlue::Direct { link_name, .. },
            ..
        } => {
            out.extend(resolve(link_name));
        }
        _ => {}
    }
}

/// Tarjan's strongly connected components, iteratively (a corpus body chain can be deep).
///
/// `order` fixes node numbering and the DFS root order; the result lists components in reverse
/// topological order of the condensation.
fn tarjan<T: Clone + Ord>(order: &[T], edges: &BTreeMap<T, BTreeSet<T>>) -> Vec<Vec<T>> {
    let index_of: BTreeMap<&T, usize> = order
        .iter()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    let adjacency: Vec<Vec<usize>> = order
        .iter()
        .map(|id| {
            edges
                .get(id)
                .into_iter()
                .flatten()
                .filter_map(|target| index_of.get(target).copied())
                .collect()
        })
        .collect();

    let n = order.len();
    const UNVISITED: usize = usize::MAX;
    let mut index = vec![UNVISITED; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut components = Vec::new();

    for root in 0..n {
        if index[root] != UNVISITED {
            continue;
        }
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(frame) = frames.last_mut() {
            let v = frame.0;
            if frame.1 < adjacency[v].len() {
                let w = adjacency[v][frame.1];
                frame.1 += 1;
                if index[w] == UNVISITED {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    frames.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                frames.pop();
                if let Some(parent) = frames.last() {
                    low[parent.0] = low[parent.0].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut component = Vec::new();
                    loop {
                        let w = stack.pop().expect("Tarjan stack holds the component");
                        on_stack[w] = false;
                        component.push(order[w].clone());
                        if w == v {
                            break;
                        }
                    }
                    component.sort();
                    components.push(component);
                }
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::semantic::DefinitionId;

    fn id(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: n,
            },
            instantiation: 0,
        }
    }

    fn edges(pairs: &[(u64, u64)]) -> BTreeMap<ItemId, BTreeSet<ItemId>> {
        let mut edges: BTreeMap<ItemId, BTreeSet<ItemId>> = BTreeMap::new();
        for (from, to) in pairs {
            edges.entry(id(*from)).or_default().insert(id(*to));
        }
        edges
    }

    #[test]
    fn a_chain_condenses_to_singletons_callees_first() {
        let order = [id(1), id(2), id(3)];
        let sccs = tarjan(&order, &edges(&[(1, 2), (2, 3)]));
        assert_eq!(sccs, vec![vec![id(3)], vec![id(2)], vec![id(1)]]);
    }

    #[test]
    fn a_cycle_is_one_component_and_sits_before_its_callers() {
        // 1 -> 2 <-> 3, 3 -> 4: the {2,3} cycle must come after 4 and before 1.
        let sccs = tarjan(
            &[id(1), id(2), id(3), id(4)],
            &edges(&[(1, 2), (2, 3), (3, 2), (3, 4)]),
        );
        assert_eq!(sccs, vec![vec![id(4)], vec![id(2), id(3)], vec![id(1)]]);
    }

    #[test]
    fn a_diamond_lists_the_join_point_first() {
        let sccs = tarjan(
            &[id(1), id(2), id(3), id(4)],
            &edges(&[(1, 2), (1, 3), (2, 4), (3, 4)]),
        );
        assert_eq!(sccs.first(), Some(&vec![id(4)]));
        assert_eq!(sccs.last(), Some(&vec![id(1)]));
        assert_eq!(sccs.len(), 4);
    }

    #[test]
    fn edges_to_missing_nodes_are_ignored_by_the_condensation() {
        let sccs = tarjan(&[id(1)], &edges(&[(1, 9)]));
        assert_eq!(sccs, vec![vec![id(1)]]);
    }

    #[test]
    fn a_self_loop_is_recursive() {
        let name = |n: u64| JsName::new(format!("f{n}"));
        let graph = Callgraph {
            callees: BTreeMap::from([(name(1), BTreeSet::from([name(1)]))]),
            sccs: vec![vec![name(1)]],
            ..Callgraph::default()
        };
        assert!(graph.recursive(&name(1)));
        assert!(!graph.recursive(&name(2)));
    }
}
