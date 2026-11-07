//! Reachability, depth first ordering and the edge sets derived from it.

use crate::cfg::{BlockId, Cfg};
use crate::StructureError;

/// Sentinel order for a block that is not reachable from the entry.
const UNREACHABLE: u32 = u32::MAX;

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
    /// Position of a block in `rpo`, or [`UNREACHABLE`].
    pub order: Vec<u32>,
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
        succs[cfg.entry.index()] = sorted_unique(cfg.blocks[cfg.entry.index()].term.successors());
        stack.push((cfg.entry, 0));
        while let Some((pc, i)) = stack.pop() {
            if i < succs[pc.index()].len() {
                let next = succs[pc.index()][i];
                stack.push((pc, i + 1));
                if !visited[next.index()] {
                    visited[next.index()] = true;
                    succs[next.index()] = sorted_unique(cfg.blocks[next.index()].term.successors());
                    stack.push((next, 0));
                }
            } else {
                post.push(pc);
            }
        }

        let mut rpo = post;
        rpo.reverse();
        let mut order = vec![UNREACHABLE; n];
        for (i, pc) in rpo.iter().enumerate() {
            order[pc.index()] = i as u32;
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

        Ok(Order { succs, preds, rpo, order })
    }

    /// Position in reverse post order.
    pub(crate) fn ord(&self, b: BlockId) -> Result<u32, StructureError> {
        match self.order.get(b.index()) {
            Some(&o) if o != UNREACHABLE => Ok(o),
            _ => Err(StructureError::UnreachableBlock(b)),
        }
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
        self.preds[b.index()].iter().any(|&p| self.is_backward(p, b))
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
    pub(crate) fn add_edge(&mut self, pred: BlockId, succ: BlockId) {
        insert_sorted(&mut self.succs[pred.index()], succ);
        insert_sorted(&mut self.preds[succ.index()], pred);
    }

    /// Sorts blocks so the latest in reverse post order comes first.
    pub(crate) fn sort_in_post_order(&self, l: &mut [BlockId]) {
        l.sort_by_key(|&b| std::cmp::Reverse(self.order.get(b.index()).copied().unwrap_or(0)));
    }
}

fn sorted_unique(mut v: Vec<BlockId>) -> Vec<BlockId> {
    v.sort_unstable();
    v.dedup();
    v
}

fn insert_sorted(v: &mut Vec<BlockId>, b: BlockId) {
    if let Err(i) = v.binary_search(&b) {
        v.insert(i, b);
    }
}
