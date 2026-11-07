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
                let left: Vec<BlockId> =
                    loops.iter().copied().filter(|h| !inner.contains(h)).collect();
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
