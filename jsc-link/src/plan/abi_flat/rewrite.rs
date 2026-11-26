//! JIR-R callee and caller rewriting for the flat direct-call ABI.

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::origin::OriginId;
use crate::jir::repr::{
    Carrier, CellDef, CellId, Func, Op, OpKind, Proj, RBlock, RBlockId, RTerm, Root, ValueDef,
    ValueId,
};
use crate::jir::semantic::{Callee, ConstTree, TypeId};
use crate::jir::types::TypeTable;

use super::plan::AbiPlan;
use super::shapes::{
    default_const_tree, is_flattenable_scalar, mask_bits, mut_enum_shape, store_view_ty,
    FlatParamKind, FlatResultKind, FlatSig, MutEnumShape, ParamAccess,
};

struct MutEnumEntry {
    orig_param: ValueId,
    shape: MutEnumShape,
    tag_param: ValueId,
    payload_params: Vec<ValueId>,
}

struct CalleeParamState {
    aggregates: BTreeMap<usize, ValueId>,
    scalar_cells: BTreeMap<usize, CellId>,
    mut_enum_entries: Vec<MutEnumEntry>,
    orig_params_by_pos: BTreeMap<usize, ValueId>,
}

/// Rewrites one callee to take its flat parameters' fields, in place.
#[cfg(test)]
pub fn rewrite_callee(func: &mut Func, sig: &FlatSig) {
    rewrite_callee_with(func, sig, None);
}

pub(super) fn rewrite_callee_with(func: &mut Func, sig: &FlatSig, types: Option<&TypeTable>) {
    let (entry_ops, params, state) = rewrite_callee_params(func, sig, types);
    func.params.clone_from(&params);
    func.blocks[0].params = params;
    let orig_ops = std::mem::take(&mut func.blocks[0].ops);
    if state.mut_enum_entries.is_empty() {
        func.blocks[0].ops = entry_ops.into_iter().chain(orig_ops).collect();
    } else {
        emit_mut_enum_entry_rebuilds(func, types, entry_ops, orig_ops, &state.mut_enum_entries);
    }

    let outs = sig.out_fields();
    if sig.result.is_some() || !outs.is_empty() {
        rewrite_callee_returns(func, sig, types, &outs, &state);
    }

    func.sig.abi.parameters = func.params.len() as u16;
    func.sig.arguments = func
        .params
        .iter()
        .map(|param| func.values[param.0 as usize].ty)
        .collect();
}

fn rewrite_callee_params(
    func: &mut Func,
    sig: &FlatSig,
    types: Option<&TypeTable>,
) -> (Vec<Op>, Vec<ValueId>, CalleeParamState) {
    let mut entry_ops = Vec::new();
    let mut params = Vec::new();
    let mut state = CalleeParamState {
        aggregates: BTreeMap::new(),
        scalar_cells: BTreeMap::new(),
        mut_enum_entries: Vec::new(),
        orig_params_by_pos: BTreeMap::new(),
    };

    for (position, param) in func.params.clone().iter().enumerate() {
        state.orig_params_by_pos.insert(position, *param);
        let Some(flat) = sig.params.get(position).and_then(Option::as_ref) else {
            params.push(*param);
            continue;
        };
        let origin = func.values[param.0 as usize].origin;
        match &flat.kind {
            FlatParamKind::Slice { .. }
            | FlatParamKind::ByteStr { .. }
            | FlatParamKind::I64Pair
            | FlatParamKind::Tagged(_)
            | FlatParamKind::Recursive(_) => {
                // Physical-only parameters remain one semantic JIR parameter; Phase C expands them.
                params.push(*param);
            }
            FlatParamKind::ScalarRef { mutable, .. } => {
                let v_in = define(func, flat.ty, origin);
                params.push(v_in);
                let cell = CellId(func.cells.len() as u32);
                func.cells.push(CellDef {
                    ty: flat.ty,
                    boxed: true,
                    origin,
                });
                entry_ops.push(Op {
                    result: None,
                    origin,
                    kind: OpKind::CellAlloc(cell),
                });
                entry_ops.push(Op {
                    result: None,
                    origin,
                    kind: OpKind::CellStore { cell, v: v_in },
                });
                entry_ops.push(Op {
                    result: Some(*param),
                    origin,
                    kind: OpKind::BorrowView {
                        root: Root::Cell(cell),
                        path: Vec::new(),
                        mutable: *mutable,
                    },
                });
                state.scalar_cells.insert(position, cell);
            }
            FlatParamKind::MutEnum { field_tys, .. } => {
                let shape = types
                    .and_then(|types| mut_enum_shape(None, types, flat.ty, Some(field_tys)))
                    .expect("mut_enum parameter shape validated before rewrite");
                let tag_param = define(func, shape.discr_ty, origin);
                params.push(tag_param);
                let payload_params: Vec<ValueId> = shape
                    .payload_tys
                    .iter()
                    .map(|ty| {
                        let value = define(func, *ty, origin);
                        params.push(value);
                        value
                    })
                    .collect();
                state.mut_enum_entries.push(MutEnumEntry {
                    orig_param: *param,
                    shape,
                    tag_param,
                    payload_params,
                });
            }
            FlatParamKind::Aggregate {
                field_tys,
                passed,
                access,
            } => {
                let fields: Vec<ValueId> = field_tys
                    .iter()
                    .enumerate()
                    .map(|(field, ty)| {
                        let value = define(func, *ty, origin);
                        if passed.contains(&(field as u32)) {
                            params.push(value);
                        } else {
                            entry_ops.push(Op {
                                result: Some(value),
                                origin,
                                kind: OpKind::Const(ConstTree::Unit),
                            });
                        }
                        value
                    })
                    .collect();
                let by_ref = !matches!(access, ParamAccess::ByValue);
                let aggregate = if by_ref {
                    define(func, flat.ty, origin)
                } else {
                    *param
                };
                entry_ops.push(Op {
                    result: Some(aggregate),
                    origin,
                    kind: OpKind::Aggregate {
                        ty: flat.ty,
                        variant: 0,
                        active_field: None,
                        fields,
                    },
                });
                if by_ref {
                    entry_ops.push(Op {
                        result: Some(*param),
                        origin,
                        kind: OpKind::BorrowView {
                            root: Root::Value(aggregate),
                            path: Vec::new(),
                            mutable: matches!(access, ParamAccess::MutRef { .. }),
                        },
                    });
                }
                state.aggregates.insert(position, aggregate);
            }
        }
    }

    (entry_ops, params, state)
}

fn emit_mut_enum_entry_rebuilds(
    func: &mut Func,
    types: Option<&TypeTable>,
    entry_ops: Vec<Op>,
    orig_ops: Vec<Op>,
    entries: &[MutEnumEntry],
) {
    let orig_term = std::mem::replace(&mut func.blocks[0].term, RTerm::Unreachable);
    func.blocks[0].ops = entry_ops;
    let mut cur_block = RBlockId(0);
    for entry in entries {
        let origin = func.values[entry.orig_param.0 as usize].origin;
        let (join, agg_init) = emit_mut_enum_rebuild_switch(
            func,
            types,
            &entry.shape,
            cur_block,
            entry.tag_param,
            &entry.payload_params,
            origin,
        );
        func.blocks[join.0 as usize].ops.push(Op {
            result: Some(entry.orig_param),
            origin,
            kind: OpKind::BorrowView {
                root: Root::Value(agg_init),
                path: Vec::new(),
                mutable: true,
            },
        });
        cur_block = join;
    }
    func.blocks[cur_block.0 as usize].ops.extend(orig_ops);
    func.blocks[cur_block.0 as usize].term = orig_term;
}

fn rewrite_callee_returns(
    func: &mut Func,
    sig: &FlatSig,
    types: Option<&TypeTable>,
    outs: &[(usize, u32)],
    state: &CalleeParamState,
) {
    let orig_block_count = func.blocks.len();
    for block in 0..orig_block_count {
        let returned = match &func.blocks[block].term {
            RTerm::Return(value) => *value,
            _ => continue,
        };
        let origin = func.blocks[block]
            .ops
            .last()
            .map(|op| op.origin)
            .unwrap_or(OriginId(0));
        let result_reads: Vec<ValueId> = match &sig.result {
            Some(result) => match &result.kind {
                FlatResultKind::Aggregate { field_tys, passed } => {
                    let base = returned.expect("a flat aggregate result returns a value");
                    passed
                        .iter()
                        .map(|&field| {
                            let value = define(func, field_tys[field as usize], origin);
                            func.blocks[block].ops.push(Op {
                                result: Some(value),
                                origin,
                                kind: OpKind::Field {
                                    base,
                                    variant: None,
                                    field,
                                },
                            });
                            value
                        })
                        .collect()
                }
                FlatResultKind::I64Pair
                | FlatResultKind::Tagged(_)
                | FlatResultKind::Recursive(_)
                | FlatResultKind::Sentinel(_) => Vec::new(),
            },
            None => Vec::new(),
        };

        let mut cur_block = RBlockId(block as u32);
        let mut out_reads: Vec<ValueId> = Vec::with_capacity(outs.len());
        for (position, param_opt) in sig.params.iter().enumerate() {
            let Some(param) = param_opt else { continue };
            match &param.kind {
                FlatParamKind::ScalarRef {
                    mutable: true,
                    written: true,
                } => {
                    let value = define(func, param.ty, origin);
                    func.blocks[cur_block.0 as usize].ops.push(Op {
                        result: Some(value),
                        origin,
                        kind: OpKind::CellLoad(state.scalar_cells[&position]),
                    });
                    out_reads.push(value);
                }
                FlatParamKind::MutEnum {
                    field_tys,
                    written: true,
                    ..
                } => {
                    let types = types.expect("mut_enum requires TypeTable");
                    let shape = mut_enum_shape(None, types, param.ty, Some(field_tys))
                        .expect("validated mut_enum shape");
                    let cur = define(func, shape.ty, origin);
                    func.blocks[cur_block.0 as usize].ops.push(Op {
                        result: Some(cur),
                        origin,
                        kind: OpKind::Deref(state.orig_params_by_pos[&position]),
                    });
                    let discr = define(func, shape.discr_ty, origin);
                    func.blocks[cur_block.0 as usize].ops.push(Op {
                        result: Some(discr),
                        origin,
                        kind: OpKind::Discriminant(cur),
                    });
                    let join = push_block(func);
                    let extracted = MutEnumEmit {
                        func,
                        types,
                        shape: &shape,
                        origin,
                    }
                    .emit_extract(cur_block, cur, discr, join);
                    cur_block = join;
                    out_reads.extend(extracted);
                }
                FlatParamKind::Aggregate {
                    field_tys,
                    passed,
                    access: ParamAccess::MutRef { written: true },
                } => {
                    let base = state.aggregates[&position];
                    for &field in passed {
                        let value = define(func, field_tys[field as usize], origin);
                        func.blocks[cur_block.0 as usize].ops.push(Op {
                            result: Some(value),
                            origin,
                            kind: OpKind::Field {
                                base,
                                variant: None,
                                field,
                            },
                        });
                        out_reads.push(value);
                    }
                }
                _ => {}
            }
        }

        let mut spilled: Vec<ValueId> = result_reads.iter().skip(1).copied().collect();
        spilled.extend(out_reads.iter().skip(usize::from(sig.returns_out_field)));
        let result_offset = sig
            .result
            .as_ref()
            .filter(|result| !result.is_aggregate())
            .map_or(0, |_| sig.result_spills());
        for (index, value) in spilled.into_iter().enumerate() {
            func.blocks[cur_block.0 as usize].ops.push(Op {
                result: None,
                origin,
                kind: OpKind::SpillWrite {
                    index: result_offset + index as u32 + 1,
                    v: value,
                },
            });
        }
        if let Some(primary) = result_reads
            .first()
            .or_else(|| sig.returns_out_field.then(|| &out_reads[0]))
        {
            func.blocks[cur_block.0 as usize].term = RTerm::Return(Some(*primary));
        } else if cur_block.0 as usize != block {
            func.blocks[cur_block.0 as usize].term = RTerm::Return(returned);
        }
    }

    if let Some(FlatResultKind::Aggregate { field_tys, passed }) =
        sig.result.as_ref().map(|r| &r.kind)
    {
        func.sig.result = field_tys[passed[0] as usize];
    } else if sig.returns_out_field {
        func.sig.result = sig.out_ty(outs[0]);
    }
}

struct MutEnumEmit<'a> {
    func: &'a mut Func,
    types: &'a TypeTable,
    shape: &'a MutEnumShape,
    origin: OriginId,
}

impl MutEnumEmit<'_> {
    /// Emits the switch + arms extracting `(tag, payload_lanes...)` from `enum_val` into `join`
    /// block parameters, returning `join`'s parameter `ValueId`s.
    fn emit_extract(
        self,
        switch_block: RBlockId,
        enum_val: ValueId,
        discr: ValueId,
        join: RBlockId,
    ) -> Vec<ValueId> {
        let k = self.shape.variants.len();
        let mut arms = Vec::with_capacity(k);
        for (variant_index, variant) in self.shape.variants.iter().enumerate() {
            let arm = push_block(self.func);
            let tag_val = define(self.func, self.shape.discr_ty, self.origin);
            self.func.blocks[arm.0 as usize].ops.push(Op {
                result: Some(tag_val),
                origin: self.origin,
                kind: OpKind::Const(ConstTree::Scalar {
                    bits: mask_bits(variant.logical_discr as u128, self.shape.discr_bytes),
                    bytes: self.shape.discr_bytes,
                }),
            });
            let mut edge_args = Vec::with_capacity(1 + self.shape.payload_tys.len());
            edge_args.push(tag_val);
            for (w_index, w_variant) in self.shape.variants.iter().enumerate() {
                for (field_index, &field_ty) in w_variant.field_tys.iter().enumerate() {
                    if self.types.is_zst(field_ty) {
                        continue;
                    }
                    let val = define(self.func, field_ty, self.origin);
                    if w_index == variant_index {
                        self.func.blocks[arm.0 as usize].ops.push(Op {
                            result: Some(val),
                            origin: self.origin,
                            kind: OpKind::Field {
                                base: enum_val,
                                variant: (variant_index != 0).then_some(variant_index as u32),
                                field: field_index as u32,
                            },
                        });
                    } else {
                        let dummy = default_const_tree(self.types, field_ty)
                            .expect("validated mut_enum field");
                        self.func.blocks[arm.0 as usize].ops.push(Op {
                            result: Some(val),
                            origin: self.origin,
                            kind: OpKind::Const(dummy),
                        });
                    }
                    edge_args.push(val);
                }
            }
            self.func.blocks[arm.0 as usize].term = RTerm::Goto(join, edge_args);
            arms.push(arm);
        }
        let cases: Vec<(u128, RBlockId, Vec<ValueId>)> = self.shape.variants[..k - 1]
            .iter()
            .enumerate()
            .map(|(variant_index, variant)| {
                (
                    mask_bits(variant.logical_discr as u128, self.shape.discr_bytes),
                    arms[variant_index],
                    Vec::new(),
                )
            })
            .collect();
        self.func.blocks[switch_block.0 as usize].term = RTerm::Switch {
            scrut: discr,
            scrut_ty: self.shape.discr_ty,
            cases,
            default: (arms[k - 1], Vec::new()),
        };
        let mut join_params = Vec::with_capacity(1 + self.shape.payload_tys.len());
        join_params.push(define(self.func, self.shape.discr_ty, self.origin));
        for &ty in &self.shape.payload_tys {
            join_params.push(define(self.func, ty, self.origin));
        }
        self.func.blocks[join.0 as usize].params = join_params.clone();
        join_params
    }
}

/// Emits a switch rebuilding an enum aggregate from `(tag_val, payload_vals)` into a fresh `join`
/// block, returning `(join_block, rebuilt_enum_value)`.
fn emit_mut_enum_rebuild_switch(
    func: &mut Func,
    types: Option<&TypeTable>,
    shape: &MutEnumShape,
    switch_block: RBlockId,
    tag_val: ValueId,
    payload_vals: &[ValueId],
    origin: OriginId,
) -> (RBlockId, ValueId) {
    let k = shape.variants.len();
    let join = push_block(func);
    let mut arms = Vec::with_capacity(k);
    let mut payload_cursor = 0usize;
    for (variant_index, variant) in shape.variants.iter().enumerate() {
        let arm = push_block(func);
        let mut fields = Vec::with_capacity(variant.field_tys.len());
        for &field_ty in &variant.field_tys {
            if types.is_some_and(|types| types.is_zst(field_ty)) {
                let unit = define(func, field_ty, origin);
                func.blocks[arm.0 as usize].ops.push(Op {
                    result: Some(unit),
                    origin,
                    kind: OpKind::Const(ConstTree::Unit),
                });
                fields.push(unit);
            } else {
                fields.push(payload_vals[payload_cursor]);
                payload_cursor += 1;
            }
        }
        let agg_v = define(func, shape.ty, origin);
        func.blocks[arm.0 as usize].ops.push(Op {
            result: Some(agg_v),
            origin,
            kind: OpKind::Aggregate {
                ty: shape.ty,
                variant: variant_index as u32,
                active_field: None,
                fields,
            },
        });
        func.blocks[arm.0 as usize].term = RTerm::Goto(join, vec![agg_v]);
        arms.push(arm);
    }
    let cases: Vec<(u128, RBlockId, Vec<ValueId>)> = shape.variants[..k - 1]
        .iter()
        .enumerate()
        .map(|(variant_index, variant)| {
            (
                mask_bits(variant.logical_discr as u128, shape.discr_bytes),
                arms[variant_index],
                Vec::new(),
            )
        })
        .collect();
    func.blocks[switch_block.0 as usize].term = RTerm::Switch {
        scrut: tag_val,
        scrut_ty: shape.discr_ty,
        cases,
        default: (arms[k - 1], Vec::new()),
    };
    let rebuilt = define(func, shape.ty, origin);
    func.blocks[join.0 as usize].params.push(rebuilt);
    (join, rebuilt)
}

fn push_block(func: &mut Func) -> RBlockId {
    let id = RBlockId(func.blocks.len() as u32);
    func.blocks.push(RBlock {
        id,
        params: Vec::new(),
        ops: Vec::new(),
        term: RTerm::Unreachable,
    });
    id
}

/// Rewrites every call `func` makes to a flat callee, in place.
///
/// `table` is the *caller's* type table, which is what the copy-back's views are typed out of.
pub fn rewrite_calls(func: &mut Func, plan: &AbiPlan, table: Option<&TypeTable>) {
    let orig_block_count = func.blocks.len();
    for block in 0..orig_block_count {
        let ops = std::mem::take(&mut func.blocks[block].ops);
        let orig_term = std::mem::replace(&mut func.blocks[block].term, RTerm::Unreachable);
        let mut cur_block = RBlockId(block as u32);
        for mut op in ops {
            let sig = match &op.kind {
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    host: None,
                    ..
                } => plan.call_signature(*item).cloned(),
                _ => None,
            };
            let (Some(sig), OpKind::Call { args, .. }) = (sig, &op.kind) else {
                func.blocks[cur_block.0 as usize].ops.push(op);
                continue;
            };
            let origin = op.origin;
            let orig_args = args.clone();
            let (flat_args, pointers) =
                rewrite_call_args(func, &mut cur_block, &sig, &orig_args, table, origin);

            let outs = sig.out_fields();
            let aggregate_result = op.result;
            let flat_primary = match (&sig.result, aggregate_result) {
                (Some(result), Some(_)) => match &result.kind {
                    FlatResultKind::Aggregate { field_tys, passed } => {
                        let value = define(func, field_tys[passed[0] as usize], origin);
                        op.result = Some(value);
                        Some(value)
                    }
                    _ => None,
                },
                (Some(result), None) if result.is_aggregate() => {
                    op.result = None;
                    None
                }
                _ => None,
            };
            if sig.result.is_none() && sig.returns_out_field {
                op.result = Some(define(func, sig.out_ty(outs[0]), origin));
            }
            let primary = op.result;
            if let OpKind::Call { args, .. } = &mut op.kind {
                *args = flat_args;
            }
            func.blocks[cur_block.0 as usize].ops.push(op);

            if let (Some(result), Some(aggregate), Some(first)) =
                (&sig.result, aggregate_result, flat_primary)
            {
                emit_aggregate_result_rebuild(func, cur_block, result, aggregate, first, origin);
            }

            if !outs.is_empty() {
                let mut copy_back = CopyBackCx {
                    func,
                    cur_block: &mut cur_block,
                    table,
                    origin,
                };
                copy_back.emit(&sig, &outs, primary, &pointers);
            }
        }
        func.blocks[cur_block.0 as usize].term = orig_term;
    }
}

fn rewrite_call_args(
    func: &mut Func,
    cur_block: &mut RBlockId,
    sig: &FlatSig,
    args: &[ValueId],
    table: Option<&TypeTable>,
    origin: OriginId,
) -> (Vec<ValueId>, BTreeMap<usize, ValueId>) {
    let mut flat_args: Vec<ValueId> = Vec::new();
    let mut pointers: BTreeMap<usize, ValueId> = BTreeMap::new();
    for (position, arg) in args.iter().enumerate() {
        let Some(flat) = sig.params.get(position).and_then(Option::as_ref) else {
            flat_args.push(*arg);
            continue;
        };
        match &flat.kind {
            FlatParamKind::Slice { .. }
            | FlatParamKind::ByteStr { .. }
            | FlatParamKind::I64Pair
            | FlatParamKind::Tagged(_)
            | FlatParamKind::Recursive(_) => {
                flat_args.push(*arg);
            }
            FlatParamKind::ScalarRef { .. } => {
                let val = define(func, flat.ty, origin);
                func.blocks[cur_block.0 as usize].ops.push(Op {
                    result: Some(val),
                    origin,
                    kind: OpKind::Deref(*arg),
                });
                flat_args.push(val);
                pointers.insert(position, *arg);
            }
            FlatParamKind::MutEnum { field_tys, .. } => {
                let types = table.expect("mut_enum requires TypeTable");
                let shape = mut_enum_shape(None, types, flat.ty, Some(field_tys))
                    .expect("validated mut_enum shape");
                let cur = define(func, shape.ty, origin);
                func.blocks[cur_block.0 as usize].ops.push(Op {
                    result: Some(cur),
                    origin,
                    kind: OpKind::Deref(*arg),
                });
                let discr = define(func, shape.discr_ty, origin);
                func.blocks[cur_block.0 as usize].ops.push(Op {
                    result: Some(discr),
                    origin,
                    kind: OpKind::Discriminant(cur),
                });
                let join = push_block(func);
                let extracted = MutEnumEmit {
                    func,
                    types,
                    shape: &shape,
                    origin,
                }
                .emit_extract(*cur_block, cur, discr, join);
                *cur_block = join;
                flat_args.extend(extracted);
                pointers.insert(position, *arg);
            }
            FlatParamKind::Aggregate {
                field_tys,
                passed,
                access,
            } => {
                let base = if matches!(access, ParamAccess::ByValue) {
                    *arg
                } else {
                    let base = define(func, flat.ty, origin);
                    func.blocks[cur_block.0 as usize].ops.push(Op {
                        result: Some(base),
                        origin,
                        kind: OpKind::Deref(*arg),
                    });
                    base
                };
                for &field in passed {
                    let ty = field_tys[field as usize];
                    let value = define(func, ty, origin);
                    func.blocks[cur_block.0 as usize].ops.push(Op {
                        result: Some(value),
                        origin,
                        kind: OpKind::Field {
                            base,
                            variant: None,
                            field,
                        },
                    });
                    flat_args.push(value);
                }
                pointers.insert(position, *arg);
            }
        }
    }
    (flat_args, pointers)
}

fn emit_aggregate_result_rebuild(
    func: &mut Func,
    cur_block: RBlockId,
    result: &super::shapes::FlatResult,
    aggregate: ValueId,
    first: ValueId,
    origin: OriginId,
) {
    let FlatResultKind::Aggregate { field_tys, passed } = &result.kind else {
        unreachable!("aggregate result rebuild called for non-aggregate result");
    };
    let mut lane = 0_u32;
    let mut fields = Vec::with_capacity(field_tys.len());
    for (field, &ty) in field_tys.iter().enumerate() {
        let field = field as u32;
        let value = if passed.contains(&field) {
            let value = if lane == 0 {
                first
            } else {
                let value = define(func, ty, origin);
                func.blocks[cur_block.0 as usize].ops.push(Op {
                    result: Some(value),
                    origin,
                    kind: OpKind::SpillRead(lane),
                });
                value
            };
            lane += 1;
            value
        } else {
            let value = define(func, ty, origin);
            func.blocks[cur_block.0 as usize].ops.push(Op {
                result: Some(value),
                origin,
                kind: OpKind::Const(ConstTree::Unit),
            });
            value
        };
        fields.push(value);
    }
    func.blocks[cur_block.0 as usize].ops.push(Op {
        result: Some(aggregate),
        origin,
        kind: OpKind::Aggregate {
            ty: result.ty,
            variant: 0,
            active_field: None,
            fields,
        },
    });
}

struct CopyBackCx<'a> {
    func: &'a mut Func,
    cur_block: &'a mut RBlockId,
    table: Option<&'a TypeTable>,
    origin: OriginId,
}

impl CopyBackCx<'_> {
    fn emit(
        &mut self,
        sig: &FlatSig,
        outs: &[(usize, u32)],
        primary: Option<ValueId>,
        pointers: &BTreeMap<usize, ValueId>,
    ) {
        let origin = self.origin;
        // Read every out-field from primary / SpillRead immediately after the Call so later
        // control flow cannot clobber spill registers, then store back per parameter.
        let out_values: Vec<ValueId> = outs
            .iter()
            .enumerate()
            .map(|(index, out_field)| match (index, sig.returns_out_field) {
                (0, true) => primary.expect("a flat return defines a value"),
                _ => {
                    let register =
                        sig.result_spills() + index as u32 + 1 - u32::from(sig.returns_out_field);
                    let value = define(self.func, sig.out_ty(*out_field), origin);
                    self.func.blocks[self.cur_block.0 as usize].ops.push(Op {
                        result: Some(value),
                        origin,
                        kind: OpKind::SpillRead(register),
                    });
                    value
                }
            })
            .collect();

        let mut out_cursor = 0usize;
        for (position, param_opt) in sig.params.iter().enumerate() {
            let Some(param) = param_opt else { continue };
            match &param.kind {
                FlatParamKind::ScalarRef {
                    mutable: true,
                    written: true,
                } => {
                    let value = out_values[out_cursor];
                    out_cursor += 1;
                    self.func.blocks[self.cur_block.0 as usize].ops.push(Op {
                        result: None,
                        origin,
                        kind: OpKind::StoreThrough {
                            ptr: pointers[&position],
                            v: value,
                        },
                    });
                }
                FlatParamKind::MutEnum {
                    field_tys,
                    written: true,
                    ..
                } => {
                    let types = self.table.expect("mut_enum requires TypeTable");
                    let shape = mut_enum_shape(None, types, param.ty, Some(field_tys))
                        .expect("validated mut_enum shape");
                    let out_tag = out_values[out_cursor];
                    let out_payloads =
                        &out_values[out_cursor + 1..out_cursor + 1 + shape.payload_tys.len()];
                    out_cursor += 1 + shape.payload_tys.len();
                    let (join, rebuilt) = emit_mut_enum_rebuild_switch(
                        self.func,
                        Some(types),
                        &shape,
                        *self.cur_block,
                        out_tag,
                        out_payloads,
                        origin,
                    );
                    self.func.blocks[join.0 as usize].ops.push(Op {
                        result: None,
                        origin,
                        kind: OpKind::StoreThrough {
                            ptr: pointers[&position],
                            v: rebuilt,
                        },
                    });
                    *self.cur_block = join;
                }
                FlatParamKind::Aggregate {
                    field_tys,
                    passed,
                    access: ParamAccess::MutRef { written: true },
                } => {
                    for &field in passed {
                        let field_ty = field_tys[field as usize];
                        let value = out_values[out_cursor];
                        out_cursor += 1;
                        let view_ty = self
                            .table
                            .and_then(|table| store_view_ty(table, field_ty))
                            .unwrap_or(field_ty);
                        let view = define(self.func, view_ty, origin);
                        self.func.blocks[self.cur_block.0 as usize].ops.push(Op {
                            result: Some(view),
                            origin,
                            kind: OpKind::BorrowView {
                                root: Root::Value(pointers[&position]),
                                path: vec![
                                    Proj::Deref,
                                    Proj::Field {
                                        field,
                                        ty: field_ty,
                                    },
                                ],
                                mutable: true,
                            },
                        });
                        self.func.blocks[self.cur_block.0 as usize].ops.push(Op {
                            result: None,
                            origin,
                            kind: OpKind::StoreThrough {
                                ptr: view,
                                v: value,
                            },
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

/// Unboxes boxed scalar cells whose entire address-taken use closure has been reduced to direct
/// `Deref`/`StoreThrough` operations after `rewrite_callee`/`rewrite_calls`.
pub(super) fn unbox_scalar_ref_cells(func: &mut Func, types: &TypeTable) {
    let mut dead_views: BTreeSet<ValueId> = BTreeSet::new();
    for cell_idx in 0..func.cells.len() {
        let cell_id = CellId(cell_idx as u32);
        let cell = func.cells[cell_idx];
        if !cell.boxed || !is_flattenable_scalar(types, cell.ty) {
            continue;
        }
        let mut views = BTreeSet::new();
        let mut cell_ok = true;
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::BorrowView {
                        root: Root::Cell(c),
                        path,
                        ..
                    } if *c == cell_id => {
                        if !path.is_empty() {
                            cell_ok = false;
                            break;
                        }
                        if let Some(result) = op.result {
                            views.insert(result);
                        } else {
                            cell_ok = false;
                            break;
                        }
                    }
                    OpKind::RawView {
                        root: Root::Cell(c),
                        ..
                    }
                    | OpKind::Drop {
                        root: Root::Cell(c),
                        ..
                    } if *c == cell_id => {
                        cell_ok = false;
                        break;
                    }
                    _ => {}
                }
            }
            if !cell_ok {
                break;
            }
        }
        if !cell_ok || views.is_empty() {
            continue;
        }
        loop {
            let before = views.len();
            for block in &func.blocks {
                for op in &block.ops {
                    match &op.kind {
                        OpKind::Copy(src) | OpKind::Move(src) if views.contains(src) => {
                            if let Some(result) = op.result {
                                views.insert(result);
                            }
                        }
                        OpKind::BorrowView {
                            root: Root::Value(src),
                            path,
                            ..
                        } if views.contains(src) && path.as_slice() == [Proj::Deref] => {
                            if let Some(result) = op.result {
                                views.insert(result);
                            }
                        }
                        _ => {}
                    }
                }
            }
            if views.len() == before {
                break;
            }
        }

        let mut uses_ok = true;
        for block in &func.blocks {
            if block.term.uses().iter().any(|u| views.contains(u)) {
                uses_ok = false;
                break;
            }
            for op in &block.ops {
                let uses_view = op.kind.uses().iter().any(|u| views.contains(u));
                if !uses_view {
                    continue;
                }
                let allowed = match &op.kind {
                    OpKind::Deref(ptr) if views.contains(ptr) => true,
                    OpKind::StoreThrough { ptr, v }
                        if views.contains(ptr) && !views.contains(v) =>
                    {
                        true
                    }
                    OpKind::Copy(src) | OpKind::Move(src)
                        if views.contains(src) && op.result.is_some() =>
                    {
                        true
                    }
                    OpKind::BorrowView {
                        root: Root::Value(src),
                        path,
                        ..
                    } if views.contains(src)
                        && path.as_slice() == [Proj::Deref]
                        && op.result.is_some() =>
                    {
                        true
                    }
                    _ => false,
                };
                if !allowed {
                    uses_ok = false;
                    break;
                }
            }
            if !uses_ok {
                break;
            }
        }
        if !uses_ok {
            continue;
        }

        func.cells[cell_idx].boxed = false;
        for block in &mut func.blocks {
            let mut new_ops = Vec::with_capacity(block.ops.len());
            for mut op in std::mem::take(&mut block.ops) {
                if let Some(r) = op.result.filter(|r| views.contains(r)) {
                    dead_views.insert(r);
                    continue;
                }
                match &op.kind {
                    OpKind::Deref(ptr) if views.contains(ptr) => {
                        op.kind = OpKind::CellLoad(cell_id);
                    }
                    OpKind::StoreThrough { ptr, v } if views.contains(ptr) => {
                        op.kind = OpKind::CellStore {
                            cell: cell_id,
                            v: *v,
                        };
                    }
                    _ => {}
                }
                new_ops.push(op);
            }
            block.ops = new_ops;
        }
    }
    crate::lower::ssa::compact_values(func, &dead_views);
}

/// Defines a fresh SSA value of type `ty`. The caller owes it exactly one definition site.
fn define(func: &mut Func, ty: TypeId, origin: OriginId) -> ValueId {
    let id = ValueId(func.values.len() as u32);
    func.values.push(ValueDef {
        ty,
        carrier: Carrier::Baseline,
        origin,
    });
    id
}
