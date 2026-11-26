//! Reachability, depth first ordering and the edge sets derived from it.

use crate::cfg::{BlockId, Cfg, Term};
use crate::StructureError;

/// The traversal order of a graph, plus its edges.
///
/// Only blocks reachable from the entry take part. Edges are stored as sorted,
/// duplicate free lists, so two branches of a `Cond` to the same target count
/// as one edge just as they do in the emitted code.
pub(crate) struct Order {
    pub succs: Vec<Vec<BlockId>>,
    pub preds: Vec<Vec<BlockId>>,
    /// Reachable blocks, entry first, in reverse post order.
    pub rpo: Vec<BlockId>,
    /// Position of a block in `rpo`, or `None` when not reachable from entry.
    pub order: Vec<Option<u32>>,
}

impl Order {
    /// Walks the graph from the entry, dropping whatever it does not reach.
    pub(crate) fn build<S, C>(cfg: &Cfg<S, C>) -> Result<Order, StructureError> {
        let n = cfg.blocks.len();
        if n == 0 || cfg.entry.index() >= n {
            return Err(StructureError::EmptyCfg);
        }
        for (i, b) in cfg.blocks.iter().enumerate() {
            for s in b.term.successors() {
                if s.index() >= n {
                    return Err(StructureError::BlockOutOfRange {
                        from: Some(BlockId(i as u32)),
                        target: s,
                    });
                }
            }
        }

        let mut succs: Vec<Vec<BlockId>> = vec![Vec::new(); n];
        let mut visited = vec![false; n];
        let mut post = Vec::with_capacity(n);
        // An explicit stack of (block, index of the next successor to visit)
        // reproduces the recursive traversal without its depth limit.
        let mut stack: Vec<(BlockId, usize)> = Vec::with_capacity(n);

        visited[cfg.entry.index()] = true;
        succs[cfg.entry.index()] = live_successors(&cfg.blocks[cfg.entry.index()].term);
        stack.push((cfg.entry, 0));
        while let Some((pc, i)) = stack.pop() {
            if i < succs[pc.index()].len() {
                let next = succs[pc.index()][i];
                stack.push((pc, i + 1));
                if !visited[next.index()] {
                    visited[next.index()] = true;
                    succs[next.index()] = live_successors(&cfg.blocks[next.index()].term);
                    stack.push((next, 0));
                }
            } else {
                post.push(pc);
            }
        }

        let mut rpo = post;
        rpo.reverse();
        let mut order = vec![None; n];
        for (i, pc) in rpo.iter().enumerate() {
            order[pc.index()] = Some(i as u32);
        }

        let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
        for &pc in &rpo {
            for &s in &succs[pc.index()] {
                preds[s.index()].push(pc);
            }
        }
        for p in preds.iter_mut() {
            p.sort_unstable();
            p.dedup();
        }

        Ok(Order {
            succs,
            preds,
            rpo,
            order,
        })
    }

    /// Position in reverse post order.
    pub(crate) fn ord(&self, b: BlockId) -> Result<u32, StructureError> {
        self.order
            .get(b.index())
            .copied()
            .flatten()
            .ok_or(StructureError::UnreachableBlock(b))
    }

    /// `a` comes before `b`: the edge between them is not a back edge.
    pub(crate) fn is_forward(&self, a: BlockId, b: BlockId) -> bool {
        match (self.ord(a), self.ord(b)) {
            (Ok(x), Ok(y)) => x < y,
            _ => false,
        }
    }

    /// `a` comes at or after `b`: an edge from `a` to `b` closes a cycle.
    pub(crate) fn is_backward(&self, a: BlockId, b: BlockId) -> bool {
        match (self.ord(a), self.ord(b)) {
            (Ok(x), Ok(y)) => x >= y,
            _ => false,
        }
    }

    /// The block is entered by a back edge, so it heads a loop.
    pub(crate) fn is_loop_header(&self, b: BlockId) -> bool {
        self.preds[b.index()]
            .iter()
            .any(|&p| self.is_backward(p, b))
    }

    /// At least two forward edges move into the block, so control joins here.
    pub(crate) fn is_merge_node(&self, b: BlockId) -> bool {
        self.preds[b.index()]
            .iter()
            .filter(|&&p| self.is_forward(p, b))
            .take(2)
            .count()
            == 2
    }

    /// Records an edge that constrains block placement without being a real
    /// transfer of control. Used by `shrink_loops`.
    ///
    /// Returns `true` if the edge was not already present.
    pub(crate) fn add_edge(&mut self, pred: BlockId, succ: BlockId) -> bool {
        let added_succ = insert_sorted(&mut self.succs[pred.index()], succ);
        let added_pred = insert_sorted(&mut self.preds[succ.index()], pred);
        added_succ || added_pred
    }

    /// Sorts blocks so the latest in reverse post order comes first.
    ///
    /// Unreachable blocks (`None`) sort strictly after every reachable block,
    /// including entry `Some(0)`.
    pub(crate) fn sort_in_post_order(&self, l: &mut [BlockId]) {
        l.sort_by_key(|&b| std::cmp::Reverse(self.order.get(b.index()).copied().flatten()));
    }
}

/// Targets a terminator can actually transfer to at runtime.
///
/// For [`Term::Switch`], duplicate case values are deduplicated keeping the
/// first target listed — matching [`crate::dtree::build_switch`] so shadowed
/// arms do not introduce unreachable phantom edges.
fn live_successors<C>(term: &Term<C>) -> Vec<BlockId> {
    match term {
        Term::Switch { cases, default, .. } => {
            let mut sorted: Vec<(u128, BlockId)> = cases.clone();
            sorted.sort_by_key(|(v, _)| *v);
            sorted.dedup_by_key(|(v, _)| *v);
            let mut targets: Vec<BlockId> = sorted.into_iter().map(|(_, b)| b).collect();
            targets.push(*default);
            sorted_unique(targets)
        }
        _ => sorted_unique(term.successors()),
    }
}

fn sorted_unique(mut v: Vec<BlockId>) -> Vec<BlockId> {
    v.sort_unstable();
    v.dedup();
    v
}

fn insert_sorted(v: &mut Vec<BlockId>, b: BlockId) -> bool {
    match v.binary_search(&b) {
        Ok(_) => false,
        Err(i) => {
            v.insert(i, b);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Block;

    #[test]
    fn sort_in_post_order_places_unreachable_after_entry() {
        let cfg: Cfg<(), ()> = Cfg {
            entry: BlockId(0),
            blocks: vec![
                Block {
                    stmts: (),
                    term: Term::Goto(BlockId(1)),
                    unwind: None,
                },
                Block {
                    stmts: (),
                    term: Term::Return,
                    unwind: None,
                },
                Block {
                    stmts: (),
                    term: Term::Return,
                    unwind: None,
                },
            ],
        };
        let order = Order::build(&cfg).unwrap();
        assert_eq!(order.order, vec![Some(0), Some(1), None]);
        assert_eq!(
            order.ord(BlockId(2)),
            Err(StructureError::UnreachableBlock(BlockId(2)))
        );

        let mut blocks = [BlockId(2), BlockId(0), BlockId(1)];
        order.sort_in_post_order(&mut blocks);
        assert_eq!(blocks, [BlockId(1), BlockId(0), BlockId(2)]);
    }

    #[test]
    fn shadowed_switch_target_is_not_a_live_edge_but_still_validated() {
        let cfg: Cfg<(), ()> = Cfg {
            entry: BlockId(0),
            blocks: vec![
                Block {
                    stmts: (),
                    term: Term::Switch {
                        on: (),
                        cases: vec![(1, BlockId(1)), (1, BlockId(2))],
                        default: BlockId(3),
                    },
                    unwind: None,
                },
                Block {
                    stmts: (),
                    term: Term::Return,
                    unwind: None,
                },
                Block {
                    stmts: (),
                    term: Term::Return,
                    unwind: None,
                },
                Block {
                    stmts: (),
                    term: Term::Return,
                    unwind: None,
                },
            ],
        };
        let order = Order::build(&cfg).unwrap();
        assert_eq!(order.order[2], None);
        assert!(order.order[1].is_some());
        assert!(order.order[3].is_some());

        let bad: Cfg<(), ()> = Cfg {
            entry: BlockId(0),
            blocks: vec![
                Block {
                    stmts: (),
                    term: Term::Switch {
                        on: (),
                        cases: vec![(1, BlockId(1)), (1, BlockId(99))],
                        default: BlockId(1),
                    },
                    unwind: None,
                },
                Block {
                    stmts: (),
                    term: Term::Return,
                    unwind: None,
                },
            ],
        };
        assert_eq!(
            Order::build(&bad).err(),
            Some(StructureError::BlockOutOfRange {
                from: Some(BlockId(0)),
                target: BlockId(99),
            })
        );
    }
}
