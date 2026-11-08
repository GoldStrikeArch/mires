//! Loop membership, reducibility, and hoisting large loop continuations out.

use crate::cfg::{BlockId, Cfg};
use crate::dom::Dom;
use crate::order::Order;
use crate::{Options, Payload, StructureError};

/// Bound on the blocks one `measure` walk looks at. The walk follows forward
/// edges only, but a diamond shaped region can still be visited along many
/// paths, so this keeps it linear enough.
const MEASURE_BUDGET: u32 = 4096;

/// Whether every back edge lands on a block that dominates its source.
///
/// A graph that fails this has a loop with more than one entry, which no
/// arrangement of loops and labeled blocks can express.
pub(crate) fn is_reducible(order: &Order, dom: &Dom) -> bool {
    for &b in &order.rpo {
        for &s in &order.succs[b.index()] {
            if order.is_backward(b, s) && !dom.dominates(order, s, b) {
                return false;
            }
        }
    }
    true
}

/// Dominator children of `parent` that lie on a cycle crossing more than one
/// sibling subtree.
///
/// An edge into a dominator subtree from outside it can only target the
/// subtree's root: a path taking it to any deeper block would avoid the root,
/// contradicting the root's dominance. Contracting each child's subtree to its
/// root therefore keeps every cycle among the siblings visible, and a
/// contracted cycle through two or more children is exactly a loop with more
/// than one entry, because no sibling dominates another. Such a loop is what
/// makes a graph irreducible, so the entries have to become the cases of a
/// dispatch. A cycle inside a single subtree is that child's own loop and
/// stays out of the result; so does an edge back to `parent`, which is
/// `parent`'s own loop.
///
/// Every entry of the cycle shows up here: an entry's immediate dominator is
/// always outside the cycle and shared with the other entries, so all of them
/// are children of the same `parent`. The members returned beyond the entries
/// are the children control re-joins from several entries, which need a case
/// for the same reason.
pub(crate) fn cyclic_children(order: &Order, dom: &Dom, parent: BlockId) -> Vec<BlockId> {
    let children = &dom.children[parent.index()];
    if children.len() < 2 {
        return Vec::new();
    }

    // Which child's subtree each block belongs to.
    const NONE: u32 = u32::MAX;
    let mut owner: Vec<u32> = vec![NONE; order.order.len()];
    let mut stack: Vec<BlockId> = Vec::new();
    for (i, &child) in children.iter().enumerate() {
        stack.push(child);
        while let Some(b) = stack.pop() {
            owner[b.index()] = i as u32;
            stack.extend_from_slice(&dom.children[b.index()]);
        }
    }

    // The contracted graph: an edge wherever a jump crosses subtrees.
    let mut succs: Vec<Vec<u32>> = vec![Vec::new(); children.len()];
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); children.len()];
    for &b in &order.rpo {
        let from = owner[b.index()];
        if from == NONE {
            continue;
        }
        for &s in &order.succs[b.index()] {
            let to = owner[s.index()];
            if to != NONE && to != from {
                succs[from as usize].push(to);
                preds[to as usize].push(from);
            }
        }
    }

    // Kosaraju: a forward pass for finish order, a backward pass to group the
    // strongly connected components.
    let mut finish: Vec<u32> = Vec::with_capacity(children.len());
    let mut seen = vec![false; children.len()];
    for root in 0..children.len() {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut walk: Vec<(u32, usize)> = vec![(root as u32, 0)];
        while let Some((node, i)) = walk.pop() {
            match succs[node as usize].get(i) {
                Some(&next) => {
                    walk.push((node, i + 1));
                    if !seen[next as usize] {
                        seen[next as usize] = true;
                        walk.push((next, 0));
                    }
                }
                None => finish.push(node),
            }
        }
    }
    let mut component = vec![NONE; children.len()];
    let mut current = 0;
    for &root in finish.iter().rev() {
        if component[root as usize] != NONE {
            continue;
        }
        component[root as usize] = current;
        let mut walk = vec![root];
        while let Some(node) = walk.pop() {
            for &p in &preds[node as usize] {
                if component[p as usize] == NONE {
                    component[p as usize] = current;
                    walk.push(p);
                }
            }
        }
        current += 1;
    }

    let mut sizes = vec![0u32; current as usize];
    for &c in &component {
        sizes[c as usize] += 1;
    }
    children
        .iter()
        .enumerate()
        .filter(|(i, _)| sizes[component[*i] as usize] >= 2)
        .map(|(_, &child)| child)
        .collect()
}

/// For each block, the headers of the loops it belongs to.
pub(crate) fn mark_loops(order: &Order) -> Vec<Vec<BlockId>> {
    let n = order.order.len();
    let mut in_loop: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    let mut work: Vec<BlockId> = Vec::new();
    for &header in &order.rpo {
        for i in 0..order.preds[header.index()].len() {
            let p = order.preds[header.index()][i];
            if !order.is_backward(p, header) {
                continue;
            }
            // Walk backwards from the back edge source; every block that
            // reaches it without passing the header is in the loop.
            work.push(p);
            while let Some(x) = work.pop() {
                if in_loop[x.index()].contains(&header) {
                    continue;
                }
                in_loop[x.index()].push(header);
                if x != header {
                    work.extend_from_slice(&order.preds[x.index()]);
                }
            }
        }
    }
    in_loop
}

/// Whether the region dominated by `root` and starting at `pc` is small enough
/// to be duplicated into a loop body.
///
/// A nested loop is never small: unrolling one into a loop body is what this
/// check exists to prevent.
fn is_small<S: Payload, C>(
    cfg: &Cfg<S, C>,
    order: &Order,
    dom: &Dom,
    root: BlockId,
    pc: BlockId,
    limit: i32,
) -> bool {
    let mut limit = limit;
    let mut steps = 0;
    let mut stack = vec![pc];
    while let Some(b) = stack.pop() {
        steps += 1;
        if steps > MEASURE_BUDGET {
            return false;
        }
        if !dom.dominates(order, root, b) {
            continue;
        }
        if order.is_loop_header(b) {
            return false;
        }
        let Some(block) = cfg.blocks.get(b.index()) else {
            return false;
        };
        limit = limit.saturating_sub(block.stmts.cost());
        if limit < 0 {
            return false;
        }
        stack.extend_from_slice(&order.succs[b.index()]);
    }
    limit >= 0
}

/// Pushes loop continuations that are too large to sit inside the loop out of
/// it.
///
/// When a block leaves a loop and is not small, an edge is added from the
/// forward predecessors of the loop header to it. The edge carries no control
/// flow; it moves the block's immediate dominator out of the loop, so the next
/// dominator tree puts it after the loop instead of inside it.
///
/// The caller must rebuild the dominator tree afterwards.
pub(crate) fn shrink_loops<S: Payload, C>(
    cfg: &Cfg<S, C>,
    order: &mut Order,
    opts: &Options,
) -> Result<(), StructureError> {
    let in_loop = mark_loops(order);
    let dom = Dom::build(order)?;
    let root = *order.rpo.first().ok_or(StructureError::EmptyCfg)?;

    let mut stack = vec![root];
    while let Some(pc) = stack.pop() {
        let loops = in_loop[pc.index()].clone();
        for &child in dom.children[pc.index()].iter() {
            if !loops.is_empty() {
                let inner = &in_loop[child.index()];
                let left: Vec<BlockId> = loops
                    .iter()
                    .copied()
                    .filter(|h| !inner.contains(h))
                    .collect();
                for header in left {
                    if is_small(cfg, order, &dom, header, child, opts.small_limit) {
                        continue;
                    }
                    let entries: Vec<BlockId> = order.preds[header.index()]
                        .iter()
                        .copied()
                        .filter(|&p| order.is_forward(p, header))
                        .collect();
                    for p in entries {
                        order.add_edge(p, child);
                    }
                }
            }
            stack.push(child);
        }
    }
    Ok(())
}
