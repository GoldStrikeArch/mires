//! The JIR-R function as a control flow graph the structurizer can read (v1 `cfg_adapter.rs`).
//!
//! Every block of the function contributes one structurizer block whose payload lists its ops
//! and whose terminator is the JIR-R one: a `Goto`, a `Cond` for a switch on `bool` (MIR lists
//! the `false` arm as value 0 and JavaScript's `switch` would compare it with `===`), a `Switch`
//! with sign-biased case values (`plan::encode`), a `Return`, an `Unreachable`, or a `Throw`.
//! Blocks the entry does not reach (cleanup blocks among them) are dropped by the structurizer.
//!
//! # Edge copies
//!
//! A block argument list is a run of assignments (`Plan::copies`) that has to run on its edge.
//! A `Goto` block carries its own edge, so its copies run at the end of its payload. A switch
//! edge with copies gets a block of its own — no ops, the edge, a `Goto` to the target — which
//! is exactly the critical-edge split out-of-SSA needs, done in the structurizer's graph rather
//! than the function's.
//!
//! # Fallback
//!
//! `structurizer::structure` fails only for a graph it cannot read; v1 retried once with
//! `force_dispatch`, and so does this. A second failure is a `Decline` (never a panic).

#![allow(dead_code)]

use std::collections::BTreeSet;

use structurizer::{Block, BlockId, Cfg, Options, Term};

use crate::jir::decline::{Decline, LowerDecline, OpClass};
use crate::jir::repr::{Func, RBlockId, RTerm};
use crate::jir::types::{TypeKind, TypeTable};

use super::plan::{encode, BlockPayload, OpRef, Plan, RScrut, Structured};

/// Structures `func` under `plan`. `opts` is v1's `js-structure=trampoline` (`force_dispatch`)
/// when set, the default options otherwise.
pub fn structure(
    func: &Func,
    plan: &Plan,
    table: &TypeTable,
    force_dispatch: bool,
) -> Result<Structured, Decline> {
    let cfg = build_cfg(func, plan, table)?;
    let options = Options {
        force_dispatch,
        ..Options::default()
    };
    match structurizer::structure(cfg, &options) {
        Ok(structured) => Ok(Structured {
            region: structured.region,
            stats: structured.stats,
        }),
        Err(first) => {
            if force_dispatch {
                return Err(decline(format!("structurizer: {first}")));
            }
            let cfg = build_cfg(func, plan, table)?;
            let options = Options {
                force_dispatch: true,
                ..Options::default()
            };
            match structurizer::structure(cfg, &options) {
                Ok(structured) => Ok(Structured {
                    region: structured.region,
                    stats: structured.stats,
                }),
                Err(second) => Err(decline(format!(
                    "structurizer: {first}; as a dispatch loop: {second}"
                ))),
            }
        }
    }
}

fn decline(detail: String) -> Decline {
    Decline::Lower(LowerDecline::Other {
        op: OpClass::Region,
        detail,
    })
}

/// The structurizer's graph: the function's blocks by number, then one split block per switch
/// edge that carries copies.
fn build_cfg(
    func: &Func,
    plan: &Plan,
    table: &TypeTable,
) -> Result<Cfg<BlockPayload, RScrut>, Decline> {
    let n = func.blocks.len();
    if n == 0 {
        return Err(decline("function has no blocks".to_owned()));
    }
    let mut blocks: Vec<Block<BlockPayload, RScrut>> = Vec::with_capacity(n);
    let mut splits: Vec<Block<BlockPayload, RScrut>> = Vec::new();
    for block in &func.blocks {
        let ops: Vec<OpRef> = (0..block.ops.len())
            .map(|index| OpRef {
                block: block.id,
                index: index as u32,
            })
            .collect();
        let has_copies = |target: RBlockId| {
            plan.copies
                .get(&(block.id, target))
                .is_some_and(|copies| !copies.is_empty())
        };
        let (edge, term) = match &block.term {
            RTerm::Goto(target, _) => (
                has_copies(*target).then_some((block.id, *target)),
                Term::Goto(BlockId(target.0)),
            ),
            RTerm::Switch {
                scrut,
                scrut_ty,
                cases,
                default,
            } => {
                let scrutinee = plan
                    .scrutinees
                    .get(&block.id)
                    .copied()
                    .unwrap_or(RScrut::Value {
                        value: *scrut,
                        ty: *scrut_ty,
                        tag_enum: None,
                    });
                // A switch edge with copies goes through a split block; the same target from
                // several cases shares one.
                let mut split_of: Vec<(RBlockId, BlockId)> = Vec::new();
                let mut seen: BTreeSet<RBlockId> = BTreeSet::new();
                let mut edge_block =
                    |target: RBlockId, splits: &mut Vec<Block<BlockPayload, RScrut>>| -> BlockId {
                        if !has_copies(target) {
                            return BlockId(target.0);
                        }
                        if let Some((_, split)) = split_of.iter().find(|(t, _)| *t == target) {
                            return *split;
                        }
                        let id = BlockId((n + splits.len()) as u32);
                        splits.push(Block {
                            stmts: BlockPayload {
                                block: None,
                                ops: Vec::new(),
                                edge: Some((block.id, target)),
                            },
                            term: Term::Goto(BlockId(target.0)),
                            unwind: None,
                        });
                        seen.insert(target);
                        split_of.push((target, id));
                        id
                    };
                let is_bool = table
                    .get(*scrut_ty)
                    .is_some_and(|descriptor| matches!(descriptor.kind, TypeKind::Bool));
                if is_bool {
                    let target_for = |wanted: u128| {
                        cases
                            .iter()
                            .find(|(value, _, _)| *value == wanted)
                            .map_or(default.0, |(_, target, _)| *target)
                    };
                    let then_ = edge_block(target_for(1), &mut splits);
                    let else_ = edge_block(target_for(0), &mut splits);
                    (
                        None,
                        Term::Cond {
                            on: scrutinee,
                            then_,
                            else_,
                        },
                    )
                } else {
                    let mut encoded = Vec::with_capacity(cases.len());
                    for (value, target, _) in cases {
                        let id = edge_block(*target, &mut splits);
                        encoded.push((encode(table, *scrut_ty, *value), id));
                    }
                    let default = edge_block(default.0, &mut splits);
                    (
                        None,
                        Term::Switch {
                            on: scrutinee,
                            cases: encoded,
                            default,
                        },
                    )
                }
            }
            RTerm::Invoke { normal, unwind } => {
                let mut targets = Vec::with_capacity(2);
                for (target, _) in [normal, unwind] {
                    if has_copies(*target) {
                        let id = BlockId((n + splits.len()) as u32);
                        splits.push(Block {
                            stmts: BlockPayload {
                                block: None,
                                ops: Vec::new(),
                                edge: Some((block.id, *target)),
                            },
                            term: Term::Goto(BlockId(target.0)),
                            unwind: None,
                        });
                        targets.push(id);
                    } else {
                        targets.push(BlockId(target.0));
                    }
                }
                (
                    None,
                    Term::Cond {
                        on: RScrut::Invoke(block.id),
                        then_: targets[0],
                        else_: targets[1],
                    },
                )
            }
            RTerm::Return(_) => (None, Term::Return),
            RTerm::Unreachable => (None, Term::Unreachable),
            RTerm::Throw(_) => (None, Term::Throw),
        };
        blocks.push(Block {
            stmts: BlockPayload {
                block: Some(block.id),
                ops,
                edge,
            },
            term,
            unwind: None,
        });
    }
    blocks.extend(splits);
    Ok(Cfg {
        blocks,
        entry: BlockId(0),
    })
}
