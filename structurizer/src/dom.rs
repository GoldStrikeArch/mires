//! Dominators, by the Cooper, Harvey and Kennedy algorithm.

use crate::cfg::BlockId;
use crate::order::Order;
use crate::StructureError;

/// Bound on the number of passes over the graph. A run needs one pass per
/// nesting level of the irreducible parts, so this is never hit in practice;
/// it only keeps a corrupt graph from spinning.
const MAX_PASSES: usize = 1000;

/// The dominator tree of a graph.
pub(crate) struct Dom {
    /// Immediate dominator of each reachable block. The entry dominates
    /// itself.
    pub idom: Vec<Option<BlockId>>,
    /// Blocks immediately dominated by each block, in ascending order.
    pub children: Vec<Vec<BlockId>>,
}

impl Dom {
    /// Iterates to a fixed point, so the result is the real dominator tree
    /// even when the graph is irreducible. One pass would be enough for a
    /// reducible graph.
    pub(crate) fn build(order: &Order) -> Result<Dom, StructureError> {
        let n = order.order.len();
        let mut idom: Vec<Option<BlockId>> = vec![None; n];
        let entry = *order.rpo.first().ok_or(StructureError::EmptyCfg)?;
        idom[entry.index()] = Some(entry);

        let mut passes = 0;
        loop {
            let mut changed = false;
            for &b in order.rpo.iter().skip(1) {
                let mut new: Option<BlockId> = None;
                for &p in &order.preds[b.index()] {
                    if idom[p.index()].is_none() {
                        continue;
                    }
                    new = Some(match new {
                        None => p,
                        Some(cur) => intersect(order, &idom, p, cur)?,
                    });
                }
                if new.is_some() && new != idom[b.index()] {
                    idom[b.index()] = new;
                    changed = true;
                }
            }
            passes += 1;
            if !changed {
                break;
            }
            if passes > MAX_PASSES {
                return Err(StructureError::Internal("dominators did not converge"));
            }
        }

        let mut children = vec![Vec::new(); n];
        for &b in order.rpo.iter().skip(1) {
            match idom[b.index()] {
                Some(p) if p != b => children[p.index()].push(b),
                _ => return Err(StructureError::Internal("block without a dominator")),
            }
        }
        for c in children.iter_mut() {
            c.sort_unstable();
        }
        Ok(Dom { idom, children })
    }

    /// Whether `a` dominates `b`.
    pub(crate) fn dominates(&self, order: &Order, a: BlockId, b: BlockId) -> bool {
        let (Ok(oa), Ok(mut ob)) = (order.ord(a), order.ord(b)) else {
            return false;
        };
        let mut x = b;
        // The immediate dominator is always earlier in reverse post order, so
        // walking up terminates.
        while ob > oa {
            match self.idom.get(x.index()).copied().flatten() {
                Some(p) if p != x => match order.ord(p) {
                    Ok(op) if op < ob => {
                        x = p;
                        ob = op;
                    }
                    _ => return false,
                },
                _ => return false,
            }
        }
        x == a
    }
}

/// The closest common dominator of two blocks, walking both up the tree built
/// so far.
fn intersect(
    order: &Order,
    idom: &[Option<BlockId>],
    mut a: BlockId,
    mut b: BlockId,
) -> Result<BlockId, StructureError> {
    let mut steps = 0;
    while a != b {
        steps += 1;
        if steps > 4 * idom.len() + 8 {
            return Err(StructureError::Internal("dominator walk did not terminate"));
        }
        let (oa, ob) = (order.ord(a)?, order.ord(b)?);
        if oa > ob {
            a = idom[a.index()].ok_or(StructureError::Internal("missing dominator"))?;
        } else {
            b = idom[b.index()].ok_or(StructureError::Internal("missing dominator"))?;
        }
    }
    Ok(a)
}
