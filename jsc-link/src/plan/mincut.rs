//! Exact binary labeling via an s-t min cut: the port of `research/v2/representation_mincut.py`
//! (Phase 4, D3).
//!
//! The convention is the Python reference's exactly: **source side is native, sink side is
//! baseline**. A node's baseline cost is the arc `source -> node` (paid when the node labels
//! baseline and the arc is cut), its native cost the arc `node -> sink`. A pairwise term is a
//! directed arc pair: the capacity of `a -> b` is paid when `a` labels native beside `b`
//! baseline, so an asymmetric adapter is simply two different capacities
//! (Kolmogorov–Zabih; legal because both are `>= 0`, which `u64` makes trivial — the
//! submodularity check `cost(B,N) + cost(N,B) >= 0` cannot fail). [`INF`] arcs are the legality
//! boundary: an INF unary forbids a label, an INF arc pair is a must-equal edge, and an
//! implication `user native => facility native` is INF from user to facility and free back.
//!
//! Determinism: adjacency is insertion order, and the caller inserts nodes and arcs in sorted
//! order, so two identical problems solve to identical labels — including on plateaus, where the
//! label falls out of the residual graph's structure rather than an iteration order.

use std::collections::VecDeque;

/// The solver's infinity, shared with the cost model: far enough from `u64::MAX` that summing
/// a path of terms cannot wrap.
pub use crate::plan::cost::INF;

#[derive(Clone, Debug)]
struct Arc {
    to: u32,
    /// Index of the reverse arc in `graph[to]`.
    rev: u32,
    cap: u64,
}

/// A Dinic max-flow instance over `nodes` problem nodes plus the implicit source and sink.
pub struct MinCut {
    nodes: usize,
    graph: Vec<Vec<Arc>>,
    /// Every node whose unary term forbade both labels.
    infeasible: Vec<usize>,
}

impl MinCut {
    pub fn new(nodes: usize) -> MinCut {
        MinCut {
            nodes,
            graph: vec![Vec::new(); nodes + 2],
            infeasible: Vec::new(),
        }
    }

    fn source(&self) -> usize {
        self.nodes
    }

    fn sink(&self) -> usize {
        self.nodes + 1
    }

    /// One directed arc `from -> to` with a zero-capacity reverse.
    fn arc(&mut self, from: usize, to: usize, cap: u64) {
        let rev = self.graph[to].len() as u32;
        let fwd = self.graph[from].len() as u32;
        self.graph[from].push(Arc {
            to: to as u32,
            rev,
            cap,
        });
        self.graph[to].push(Arc {
            to: from as u32,
            rev: fwd,
            cap: 0,
        });
    }

    /// The unary term of `node`: `baseline` is paid when the node labels baseline, `native` when
    /// it labels native.
    ///
    /// A node with both labels at [`INF`] has no legal label. That is never a panic in the linker:
    /// [`MinCut::infeasible`] remembers the node while the residual problem forces it to baseline.
    /// Its impossible choice cannot poison an independent component while the caller prepares the
    /// region decline.
    pub fn unary(&mut self, node: usize, baseline: u64, native: u64) {
        assert!(node < self.nodes);
        let baseline = if baseline >= INF && native >= INF {
            self.infeasible.push(node);
            0
        } else {
            baseline
        };
        let (source, sink) = (self.source(), self.sink());
        self.arc(source, node, baseline);
        self.arc(node, sink, native);
    }

    /// The nodes [`MinCut::unary`] saw with both labels at [`INF`]. Their regions have no legal
    /// labeling and must be declined instead of committed.
    pub fn infeasible(&self) -> &[usize] {
        &self.infeasible
    }

    /// The pairwise term of `(a, b)`: `ab` is paid when `a` is native beside `b` baseline, `ba`
    /// the other way around.
    pub fn pair(&mut self, a: usize, b: usize, ab: u64, ba: u64) {
        assert!(a < self.nodes && b < self.nodes && a != b);
        self.arc(a, b, ab);
        self.arc(b, a, ba);
    }

    /// The minimum total cost, leaving the residual graph behind for [`MinCut::native_side`].
    pub fn solve(&mut self) -> u64 {
        let (source, sink) = (self.source(), self.sink());
        let mut total: u64 = 0;
        loop {
            let level = self.levels(source);
            if level[sink] < 0 {
                return total;
            }
            let mut cursor = vec![0usize; self.graph.len()];
            loop {
                let pushed = self.blocking_path(source, sink, &level, &mut cursor);
                if pushed == 0 {
                    break;
                }
                total = total.saturating_add(pushed);
            }
        }
    }

    /// BFS levels over positive-capacity arcs.
    fn levels(&self, source: usize) -> Vec<i32> {
        let mut level = vec![-1i32; self.graph.len()];
        level[source] = 0;
        let mut queue = VecDeque::from([source]);
        while let Some(vertex) = queue.pop_front() {
            for arc in &self.graph[vertex] {
                if arc.cap > 0 && level[arc.to as usize] < 0 {
                    level[arc.to as usize] = level[vertex] + 1;
                    queue.push_back(arc.to as usize);
                }
            }
        }
        level
    }

    /// One augmenting path along the level graph, iteratively (the Python `send`, without the
    /// recursion): walk forward through the cursors, and on reaching the sink push the path's
    /// bottleneck back along it.
    fn blocking_path(
        &mut self,
        source: usize,
        sink: usize,
        level: &[i32],
        cursor: &mut [usize],
    ) -> u64 {
        // The path as (vertex, arc index) hops.
        let mut path: Vec<(usize, usize)> = Vec::new();
        let mut vertex = source;
        loop {
            if vertex == sink {
                let bottleneck = path
                    .iter()
                    .map(|(v, i)| self.graph[*v][*i].cap)
                    .min()
                    .unwrap_or(0);
                for (v, i) in &path {
                    let (to, rev) = {
                        let arc = &mut self.graph[*v][*i];
                        arc.cap -= bottleneck;
                        (arc.to as usize, arc.rev as usize)
                    };
                    let reverse = &mut self.graph[to][rev];
                    reverse.cap = reverse.cap.saturating_add(bottleneck);
                }
                return bottleneck;
            }
            match self.graph[vertex].get(cursor[vertex]) {
                None => {
                    // Dead end: retreat, exhausting the arc that led here.
                    let Some((back, index)) = path.pop() else {
                        return 0;
                    };
                    cursor[back] = index + 1;
                    vertex = back;
                }
                Some(arc) => {
                    if arc.cap > 0 && level[arc.to as usize] == level[vertex] + 1 {
                        path.push((vertex, cursor[vertex]));
                        vertex = arc.to as usize;
                    } else {
                        cursor[vertex] += 1;
                    }
                }
            }
        }
    }

    /// After [`MinCut::solve`]: `true` for every node on the source (native) side of the cut.
    pub fn native_side(&self) -> Vec<bool> {
        let mut native = vec![false; self.graph.len()];
        native[self.source()] = true;
        let mut queue = VecDeque::from([self.source()]);
        while let Some(vertex) = queue.pop_front() {
            for arc in &self.graph[vertex] {
                if arc.cap > 0 && !native[arc.to as usize] {
                    native[arc.to as usize] = true;
                    queue.push_back(arc.to as usize);
                }
            }
        }
        native.truncate(self.nodes);
        native
    }
}

/// The reference entry point: unary cost vectors plus directed arc pairs, exactly
/// `representation_mincut.py::solve` with asymmetric edges. Returns the optimum and the labels,
/// `true` = native. The planner drives [`MinCut`] directly; this is the parity harness's shape.
#[cfg(test)]
pub fn solve(
    baseline: &[u64],
    native: &[u64],
    pairs: &[(usize, usize, u64, u64)],
) -> (u64, Vec<bool>) {
    assert_eq!(baseline.len(), native.len());
    let mut cut = MinCut::new(baseline.len());
    for (node, (b, n)) in baseline.iter().zip(native).enumerate() {
        cut.unary(node, *b, *n);
    }
    for (a, b, ab, ba) in pairs {
        cut.pair(*a, *b, *ab, *ba);
    }
    let optimum = cut.solve();
    (optimum, cut.native_side())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labeling's real cost: unaries plus every crossing arc, INF-saturating.
    fn cost_of(
        labels: &[bool],
        baseline: &[u64],
        native: &[u64],
        pairs: &[(usize, usize, u64, u64)],
    ) -> u64 {
        let mut total: u64 = 0;
        for (node, is_native) in labels.iter().enumerate() {
            total = total.saturating_add(if *is_native {
                native[node]
            } else {
                baseline[node]
            });
        }
        for (a, b, ab, ba) in pairs {
            if labels[*a] && !labels[*b] {
                total = total.saturating_add(*ab);
            }
            if labels[*b] && !labels[*a] {
                total = total.saturating_add(*ba);
            }
        }
        total
    }

    fn brute_force(baseline: &[u64], native: &[u64], pairs: &[(usize, usize, u64, u64)]) -> u64 {
        let count = baseline.len();
        let mut best = u64::MAX;
        for mask in 0..(1u32 << count) {
            let labels: Vec<bool> = (0..count).map(|node| mask & (1 << node) != 0).collect();
            best = best.min(cost_of(&labels, baseline, native, pairs));
        }
        best
    }

    /// The Python fixture reproduces exactly: `optimum=928 labels=NNNNNNNNB`.
    #[test]
    fn the_python_fixture_reproduces() {
        let baseline = [177, 617, 361, 484, 212, 180, 776, 70, 90];
        let native = [34, 92, 58, 85, 45, 40, 210, 18, INF];
        let pairs: Vec<(usize, usize, u64, u64)> = [
            (0, 6, 48),
            (1, 6, 64),
            (2, 6, 64),
            (3, 4, 32),
            (4, 5, 32),
            (5, 6, 48),
            (6, 7, 0),
            (6, 8, 256),
        ]
        .into_iter()
        .map(|(a, b, cost)| (a, b, cost, cost))
        .collect();
        let (optimum, labels) = solve(&baseline, &native, &pairs);
        assert_eq!(optimum, 928);
        let spelled: String = labels
            .iter()
            .map(|native| if *native { 'N' } else { 'B' })
            .collect();
        assert_eq!(spelled, "NNNNNNNNB");
    }

    /// Brute-force parity over a seeded corpus of graphs with up to 12 nodes: the cut's optimum
    /// equals exhaustive search, and the returned labels cost exactly the optimum.
    #[test]
    fn brute_force_parity_on_small_graphs() {
        // A tiny deterministic LCG: no dependency, same corpus every run.
        let mut state: u64 = 0x5243_474A_5342_u64;
        let mut next = move |bound: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % bound
        };
        for trial in 0..200 {
            let count = 2 + (next(11) as usize); // 2..=12
            let baseline: Vec<u64> = (0..count).map(|_| next(400)).collect();
            let mut native: Vec<u64> = (0..count).map(|_| next(400)).collect();
            // Sprinkle INF pins, never on both sides of one node.
            for cost in native.iter_mut() {
                if next(5) == 0 {
                    *cost = INF;
                }
            }
            let edges = next(2 * count as u64) as usize;
            let pairs: Vec<(usize, usize, u64, u64)> = (0..edges)
                .filter_map(|_| {
                    let a = next(count as u64) as usize;
                    let b = next(count as u64) as usize;
                    (a != b).then(|| {
                        let ab = if next(7) == 0 { INF } else { next(120) };
                        let ba = if next(7) == 0 { INF } else { next(120) };
                        (a, b, ab, ba)
                    })
                })
                .collect();
            let (optimum, labels) = solve(&baseline, &native, &pairs);
            let best = brute_force(&baseline, &native, &pairs);
            assert_eq!(optimum, best, "trial {trial} disagrees with brute force");
            assert_eq!(
                cost_of(&labels, &baseline, &native, &pairs),
                optimum,
                "trial {trial}: the labels do not cost the optimum"
            );
        }
    }

    /// An asymmetric arc pair: cutting in the cheap direction wins, and reversing the costs
    /// flips the labeling.
    #[test]
    fn asymmetric_arcs_cut_in_the_cheap_direction() {
        // Node 0 wants native (saves 100), node 1 is pinned baseline. The adapter costs 10 with
        // 0 native beside 1 baseline, and INF the other way.
        let (optimum, labels) = solve(&[100, 0], &[0, INF], &[(0, 1, 10, INF)]);
        assert_eq!(optimum, 10);
        assert_eq!(labels, [true, false]);
        // With the directions swapped the cut is unaffordable, so node 0 stays baseline.
        let (optimum, labels) = solve(&[100, 0], &[0, INF], &[(0, 1, INF, 10)]);
        assert_eq!(optimum, 100);
        assert_eq!(labels, [false, false]);
    }

    /// The facility implication: `user native => facility native` is INF from user to facility
    /// and free back, so the facility's cost is paid exactly when a user goes native.
    #[test]
    fn an_inf_implication_charges_the_facility_with_its_user() {
        // User 0 saves 100 by going native; facility 1 costs 30 native.
        let (optimum, labels) = solve(&[100, 0], &[0, 30], &[(0, 1, INF, 0)]);
        assert_eq!(optimum, 30);
        assert_eq!(labels, [true, true], "the user drags the facility native");
        // A facility too expensive to justify keeps the user baseline.
        let (optimum, labels) = solve(&[100, 0], &[0, 500], &[(0, 1, INF, 0)]);
        assert_eq!(optimum, 100);
        assert_eq!(labels, [false, false]);
    }

    /// A must-equal pair moves together whichever side is cheaper in total.
    #[test]
    fn a_must_equal_pair_moves_together() {
        let (optimum, labels) = solve(&[100, 5], &[0, 40], &[(0, 1, INF, INF)]);
        assert_eq!(optimum, 40);
        assert_eq!(labels, [true, true]);
    }

    /// Both-INF nodes never panic or poison an independent component: the solve forces them to
    /// baseline and reports all of them for their callers' region declines.
    #[test]
    fn a_both_inf_node_is_infeasible_not_a_panic() {
        let mut cut = MinCut::new(3);
        cut.unary(0, INF, INF);
        cut.unary(1, 10, 20);
        cut.unary(2, INF, INF);
        assert_eq!(cut.infeasible(), &[0, 2]);
        assert_eq!(cut.solve(), 10);
        assert_eq!(cut.native_side(), vec![false, false, false]);
    }

    /// Two identical solves label identically, plateaus included.
    #[test]
    fn the_solve_is_deterministic() {
        let baseline = [50, 50, 50, 50];
        let native = [50, 50, 50, 50];
        let pairs = [(0usize, 1usize, 0u64, 0u64), (2, 3, 10, 10)];
        let one = solve(&baseline, &native, &pairs);
        let two = solve(&baseline, &native, &pairs);
        assert_eq!(one, two);
    }
}
