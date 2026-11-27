//! Shared CFG, def-use, and scalar bit-masking helpers for the JIR-R optimizer passes.

use std::collections::BTreeSet;

use crate::jir::repr::{Dominators, Func, Op, OpKind, RBlockId, ValueId};

/// Precomputed control-flow graph structure for a [`Func`]: dominators, deduplicated
/// predecessors, and deduplicated successors.
#[derive(Clone, Debug)]
pub(crate) struct Cfg {
    pub doms: Dominators,
    /// Deduplicated predecessor blocks for each block, in ascending block-id order.
    pub preds: Vec<Vec<RBlockId>>,
    /// Deduplicated in-range successor blocks for each block, in terminator order.
    pub succs: Vec<Vec<RBlockId>>,
}

impl Cfg {
    /// Builds dominators and deduplicated predecessor/successor adjacency lists for `func`.
    pub(crate) fn of(func: &Func) -> Self {
        let doms = Dominators::of(func);
        let n = func.blocks.len();
        let mut succs: Vec<Vec<RBlockId>> = vec![Vec::new(); n];
        let mut preds: Vec<Vec<RBlockId>> = vec![Vec::new(); n];
        for (bi, block) in func.blocks.iter().enumerate() {
            let from = RBlockId(bi as u32);
            for target in block.term.successors() {
                let ti = target.0 as usize;
                if ti >= n {
                    continue;
                }
                if !succs[bi].contains(&target) {
                    succs[bi].push(target);
                    preds[ti].push(from);
                }
            }
        }
        Self { doms, preds, succs }
    }

    /// Reachable predecessors of `block`, in ascending block-id order.
    pub(crate) fn reachable_preds(&self, block: RBlockId) -> impl Iterator<Item = RBlockId> + '_ {
        self.preds[block.0 as usize]
            .iter()
            .copied()
            .filter(|&pred| self.doms.is_reachable(pred))
    }

    /// Incoming edge argument slices per block across all blocks in the function, in block and
    /// terminator edge order.
    pub(crate) fn incoming_args(func: &Func) -> Vec<Vec<&[ValueId]>> {
        let n = func.blocks.len();
        let mut incoming: Vec<Vec<&[ValueId]>> = vec![Vec::new(); n];
        for block in &func.blocks {
            for (target, args) in block.term.edges() {
                if let Some(slot) = incoming.get_mut(target.0 as usize) {
                    slot.push(args);
                }
            }
        }
        incoming
    }

    /// Incoming edge argument slices per block from reachable predecessors only, in RPO order.
    pub(crate) fn reachable_incoming_args<'a>(&self, func: &'a Func) -> Vec<Vec<&'a [ValueId]>> {
        let n = func.blocks.len();
        let mut incoming: Vec<Vec<&'a [ValueId]>> = vec![Vec::new(); n];
        for &id in &self.doms.rpo {
            for (target, args) in func.blocks[id.0 as usize].term.edges() {
                if let Some(slot) = incoming.get_mut(target.0 as usize) {
                    slot.push(args);
                }
            }
        }
        incoming
    }

    /// Incoming `(predecessor_block, edge_args)` pairs per block across all blocks in the
    /// function, in block and terminator edge order.
    pub(crate) fn incoming_edges(func: &Func) -> Vec<Vec<(RBlockId, &[ValueId])>> {
        let n = func.blocks.len();
        let mut incoming: Vec<Vec<(RBlockId, &[ValueId])>> = vec![Vec::new(); n];
        for block in &func.blocks {
            for (target, args) in block.term.edges() {
                if let Some(slot) = incoming.get_mut(target.0 as usize) {
                    slot.push((block.id, args));
                }
            }
        }
        incoming
    }

    /// The natural loop of `header`: `header` together with every block that can reach a backedge
    /// source (`pred -> header` with `header` dominating `pred`) without leaving through `header`.
    pub(crate) fn natural_loop(&self, header: RBlockId) -> BTreeSet<RBlockId> {
        let mut loop_blocks = BTreeSet::new();
        loop_blocks.insert(header);
        let mut worklist = Vec::new();
        for &pred in &self.preds[header.0 as usize] {
            if self.doms.dominates(header, pred) && loop_blocks.insert(pred) {
                worklist.push(pred);
            }
        }
        while let Some(block) = worklist.pop() {
            for &pred in &self.preds[block.0 as usize] {
                if loop_blocks.insert(pred) {
                    worklist.push(pred);
                }
            }
        }
        loop_blocks
    }

    /// Returns a boolean mask indexed by `RBlockId` marking every block that sits on a directed
    /// cycle (i.e. can reach itself via one or more CFG edges).
    ///
    /// Computed in O(V + E) via iterative Tarjan's SCC algorithm: a vertex can reach itself iff it
    /// has a self-loop or belongs to an SCC of size >= 2.
    pub(crate) fn looping_blocks(&self) -> Vec<bool> {
        let n = self.succs.len();
        let mut index_counter = 0u32;
        let mut index = vec![u32::MAX; n];
        let mut lowlink = vec![0u32; n];
        let mut on_stack = vec![false; n];
        let mut tarjan_stack: Vec<usize> = Vec::new();
        let mut call_stack: Vec<(usize, usize)> = Vec::new();
        let mut looping = vec![false; n];

        for start in 0..n {
            if index[start] != u32::MAX {
                continue;
            }
            index[start] = index_counter;
            lowlink[start] = index_counter;
            index_counter += 1;
            tarjan_stack.push(start);
            on_stack[start] = true;
            call_stack.push((start, 0));

            while let Some((v, next_edge)) = call_stack.last_mut() {
                let v = *v;
                if *next_edge < self.succs[v].len() {
                    let w = self.succs[v][*next_edge].0 as usize;
                    *next_edge += 1;
                    if w == v {
                        looping[v] = true;
                    }
                    if index[w] == u32::MAX {
                        index[w] = index_counter;
                        lowlink[w] = index_counter;
                        index_counter += 1;
                        tarjan_stack.push(w);
                        on_stack[w] = true;
                        call_stack.push((w, 0));
                    } else if on_stack[w] {
                        lowlink[v] = lowlink[v].min(index[w]);
                    }
                } else {
                    call_stack.pop();
                    if let Some(&(parent, _)) = call_stack.last() {
                        lowlink[parent] = lowlink[parent].min(lowlink[v]);
                    }
                    if lowlink[v] == index[v] {
                        let mut scc_size = 0usize;
                        let mut first = 0usize;
                        while let Some(member) = tarjan_stack.pop() {
                            on_stack[member] = false;
                            if scc_size == 0 {
                                first = member;
                            } else {
                                if scc_size == 1 {
                                    looping[first] = true;
                                }
                                looping[member] = true;
                            }
                            scc_size += 1;
                            if member == v {
                                break;
                            }
                        }
                    }
                }
            }
        }
        looping
    }

    /// Whether the CFG contains any directed cycle.
    #[allow(
        dead_code,
        reason = "shared CFG query API for optimizer passes and tests"
    )]
    pub(crate) fn has_cycle(&self) -> bool {
        self.looping_blocks().into_iter().any(|in_cycle| in_cycle)
    }
}

/// Precomputed SSA def-site and use-count index for a [`Func`].
#[derive(Clone, Debug)]
pub(crate) struct DefUse {
    /// For values defined by an op: `Some((block, op_index))`.
    pub def_op: Vec<Option<(RBlockId, usize)>>,
    /// For values defined as a block parameter: `Some((block, param_index))`.
    pub def_param: Vec<Option<(RBlockId, usize)>>,
    /// Total operand uses (across ops and terminators) per value.
    pub use_counts: Vec<u32>,
}

impl DefUse {
    /// Indexes every value definition (op or block parameter) and use count in one linear pass.
    pub(crate) fn of(func: &Func) -> Self {
        let n = func.values.len();
        let mut def_op = vec![None; n];
        let mut def_param = vec![None; n];
        let mut use_counts = vec![0u32; n];
        for (bi, block) in func.blocks.iter().enumerate() {
            let block_id = RBlockId(bi as u32);
            for (pos, &param) in block.params.iter().enumerate() {
                if let Some(slot) = def_param.get_mut(param.0 as usize) {
                    *slot = Some((block_id, pos));
                }
            }
            for (oi, op) in block.ops.iter().enumerate() {
                if let Some(result) = op.result {
                    if let Some(slot) = def_op.get_mut(result.0 as usize) {
                        *slot = Some((block_id, oi));
                    }
                }
                op.kind.for_each_use(|v| {
                    if let Some(count) = use_counts.get_mut(v.0 as usize) {
                        *count += 1;
                    }
                });
            }
            block.term.for_each_use(|v| {
                if let Some(count) = use_counts.get_mut(v.0 as usize) {
                    *count += 1;
                }
            });
        }
        Self {
            def_op,
            def_param,
            use_counts,
        }
    }

    /// The `(block, op_index)` defining `v`, if `v` is defined by an op.
    pub(crate) fn def_site(&self, v: ValueId) -> Option<(RBlockId, usize)> {
        self.def_op.get(v.0 as usize).copied().flatten()
    }

    /// The `(block, param_index)` defining `v`, if `v` is a block parameter.
    pub(crate) fn param_site(&self, v: ValueId) -> Option<(RBlockId, usize)> {
        self.def_param.get(v.0 as usize).copied().flatten()
    }

    /// The defining [`Op`] of `v`, if `v` is defined by an op.
    pub(crate) fn def_op<'a>(&self, func: &'a Func, v: ValueId) -> Option<&'a Op> {
        let (block, idx) = self.def_site(v)?;
        func.blocks.get(block.0 as usize)?.ops.get(idx)
    }

    /// The defining [`OpKind`] of `v`, if `v` is defined by an op.
    pub(crate) fn def_kind<'a>(&self, func: &'a Func, v: ValueId) -> Option<&'a OpKind> {
        self.def_op(func, v).map(|op| &op.kind)
    }

    /// Number of operand uses of `v` across ops and terminators.
    pub(crate) fn use_count(&self, v: ValueId) -> u32 {
        self.use_counts.get(v.0 as usize).copied().unwrap_or(0)
    }
}

/// `bits` truncated to `bytes` bytes; 16 or more is the identity.
pub(crate) fn mask(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1u128 << (u32::from(bytes) * 8)) - 1),
        _ => bits,
    }
}

/// `bits` truncated to `width` bits; 128 or more is the identity.
pub(crate) fn mask_bits(bits: u128, width: u32) -> u128 {
    match width {
        0 => 0,
        1..=127 => bits & ((1u128 << width) - 1),
        _ => bits,
    }
}

/// Sign-extends the low `width` bits of `bits` to `i128`.
pub(crate) fn sign_extend(bits: u128, width: u32) -> i128 {
    let masked = mask_bits(bits, width);
    if width == 0 || width >= 128 {
        masked as i128
    } else {
        let shift = 128 - width;
        ((masked << shift) as i128) >> shift
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::origin::OriginId;
    use crate::jir::repr::{FuncBuilder, RTerm};
    use crate::jir::semantic::{AbiSignature, ConstTree, DefinitionId, ItemId, Signature, TypeId};

    const I32: TypeId = TypeId(1);
    const O: OriginId = OriginId(0);

    fn builder(params: usize) -> FuncBuilder {
        FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 1,
                },
                instantiation: 0,
            },
            Signature {
                arguments: vec![I32; params],
                result: I32,
                abi: AbiSignature {
                    parameters: params as u16,
                    ..AbiSignature::default()
                },
            },
        )
    }

    #[test]
    fn scalar_masking_and_sign_extension() {
        assert_eq!(mask(0x1234_5678, 0), 0);
        assert_eq!(mask(0x1234_5678, 1), 0x78);
        assert_eq!(mask(0x1234_5678, 2), 0x5678);
        assert_eq!(mask(u128::MAX, 16), u128::MAX);
        assert_eq!(mask_bits(0b1111_0101, 4), 0b0101);
        assert_eq!(mask_bits(u128::MAX, 128), u128::MAX);
        assert_eq!(sign_extend(0xff, 8), -1);
        assert_eq!(sign_extend(0x7f, 8), 127);
        assert_eq!(sign_extend(0x8000_0000, 32), i128::from(i32::MIN));
    }

    #[test]
    fn cfg_loops_natural_loop_and_def_use() {
        // entry(b0) -> header(b1) <-> body(b2), header(b1) -> exit(b3), plus self-loop b4 -> b4.
        let mut b = builder(1);
        let entry = FuncBuilder::ENTRY;
        let header = b.block();
        let body = b.block();
        let exit = b.block();
        let dead_self = b.block();

        let p0 = b.param(I32, O);
        let c0 = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
            I32,
            O,
        );
        b.terminate(entry, RTerm::Goto(header, vec![p0]));

        let h_param = b.block_param(header, I32, O);
        b.terminate(
            header,
            RTerm::Switch {
                scrut: h_param,
                scrut_ty: I32,
                cases: vec![(0, exit, Vec::new())],
                default: (body, Vec::new()),
            },
        );
        b.terminate(body, RTerm::Goto(header, vec![c0]));
        b.terminate(exit, RTerm::Return(Some(h_param)));
        b.terminate(dead_self, RTerm::Goto(dead_self, Vec::new()));

        let func = b.finish();
        let cfg = Cfg::of(&func);
        assert!(cfg.has_cycle());
        assert_eq!(cfg.looping_blocks(), vec![false, true, true, false, true]);
        assert_eq!(cfg.natural_loop(header), BTreeSet::from([header, body]));
        assert_eq!(cfg.preds[header.0 as usize], vec![entry, body]);
        assert_eq!(
            cfg.reachable_preds(header).collect::<Vec<_>>(),
            vec![entry, body]
        );

        let du = DefUse::of(&func);
        assert_eq!(du.param_site(p0), Some((entry, 0)));
        assert_eq!(du.param_site(h_param), Some((header, 0)));
        assert_eq!(du.def_site(c0), Some((entry, 0)));
        assert_eq!(du.use_count(p0), 1);
        assert_eq!(du.use_count(h_param), 2);
        assert_eq!(du.use_count(c0), 1);
    }

    #[test]
    fn acyclic_cfg_reports_no_cycles() {
        let mut b = builder(1);
        let entry = FuncBuilder::ENTRY;
        let left = b.block();
        let right = b.block();
        let merge = b.block();
        let x = b.param(I32, O);
        b.terminate(
            entry,
            RTerm::Switch {
                scrut: x,
                scrut_ty: I32,
                cases: vec![(0, left, Vec::new())],
                default: (right, Vec::new()),
            },
        );
        b.terminate(left, RTerm::Goto(merge, vec![x]));
        b.terminate(right, RTerm::Goto(merge, vec![x]));
        let m = b.block_param(merge, I32, O);
        b.terminate(merge, RTerm::Return(Some(m)));

        let func = b.finish();
        let cfg = Cfg::of(&func);
        assert!(!cfg.has_cycle());
        assert_eq!(cfg.looping_blocks(), vec![false; 4]);
    }
}
