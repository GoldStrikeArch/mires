//! Interprocedural forwarding of aggregate return fields.
//!
//! A callee may return a product whose field is exactly one of its parameters. When the caller's
//! same block copies that product before either the result is used or an effect can mutate an
//! aliased argument, rebuilding the copy from copied fields exposes the forwarded value to the
//! ordinary scalar and range passes without inlining the callee. The call remains in place, so
//! panics, effects, and `#[track_caller]` behavior are unchanged. The use/effect boundary is what
//! keeps a later copy of a mutated result from being rebuilt from the call's original arguments.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::JsName;
use crate::jir::decline::Decline;
use crate::jir::interns::Interns;
use crate::jir::repr::{Carrier, Func, Op, OpKind, RBlockId, ValueId};
use crate::jir::semantic::{CallSemantic, Callee, TypeId};
use crate::lower::fuse::{op_purity, Purity};

use super::candidates;
use super::cfg::{Cfg, DefUse};
use super::tx::{try_rewrite_func, TxOutcome};

/// What the pass changed and proved.
#[derive(Clone, Debug, Default)]
pub struct ProjectionStats {
    pub items: BTreeSet<JsName>,
    pub witnesses_recorded: u32,
    pub witnesses_failed: u32,
}

#[derive(Clone, Debug)]
struct Contract {
    ty: TypeId,
    /// Result field -> JavaScript ABI parameter, where the callee forwards one unchanged.
    fields: Vec<Option<usize>>,
}

/// Rebuilds eligible call-result copies in every caller, transactionally per function.
pub fn run(
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &Interns>,
) -> ProjectionStats {
    let contracts: BTreeMap<JsName, Contract> = funcs
        .iter()
        .filter_map(|(name, func)| {
            let func = func.as_ref().ok()?;
            let table = &interns.get(name)?.types;
            forwarded_fields(func, table).map(|contract| (name.clone(), contract))
        })
        .collect();
    if contracts.is_empty() {
        return ProjectionStats::default();
    }

    let mut stats = ProjectionStats::default();
    let names: Vec<JsName> = funcs.keys().cloned().collect();
    for name in names {
        let Some(table) = interns.get(&name).copied() else {
            continue;
        };
        let Some(Ok(func)) = funcs.get(&name) else {
            continue;
        };
        match try_rewrite_func(
            func,
            &table.types,
            |rewritten| {
                let fields = expand_copies(rewritten, table, &contracts);
                (fields > 0).then_some(fields)
            },
            |_, _| true,
        ) {
            TxOutcome::Unchanged => {}
            TxOutcome::RolledBack => {
                stats.witnesses_failed += 1;
            }
            TxOutcome::Committed {
                func: rewritten, ..
            } => {
                stats.witnesses_recorded += 1;
                stats.items.insert(name.clone());
                funcs.insert(name, Ok(rewritten));
            }
        }
    }
    stats
}

/// Fields every normal return forwards from the same ABI parameter.
fn forwarded_fields(func: &Func, table: &crate::jir::TypeTable) -> Option<Contract> {
    let shape = candidates::shape(table, func.sig.result, 0)?;
    let def_use = DefUse::of(func);
    let resolve = |mut value: ValueId| {
        for _ in 0..8 {
            match def_use.def_kind(func, value) {
                Some(OpKind::Copy(next) | OpKind::Move(next)) => value = *next,
                _ => break,
            }
        }
        value
    };

    let mut combined: Option<Vec<Option<usize>>> = None;
    let mut returns = 0usize;
    for block in &func.blocks {
        let crate::jir::repr::RTerm::Return(Some(value)) = block.term else {
            if matches!(block.term, crate::jir::repr::RTerm::Return(None)) {
                return None;
            }
            continue;
        };
        returns += 1;
        let value = resolve(value);
        let Some(OpKind::Aggregate {
            ty,
            variant: 0,
            active_field: None,
            fields,
        }) = def_use.def_kind(func, value)
        else {
            return None;
        };
        if *ty != func.sig.result || fields.len() != shape.len() {
            return None;
        }
        let row: Vec<Option<usize>> = fields
            .iter()
            .map(|field| {
                let field = resolve(*field);
                func.params.iter().position(|param| *param == field)
            })
            .collect();
        match &mut combined {
            None => combined = Some(row),
            Some(known) => {
                for (known, returned) in known.iter_mut().zip(row) {
                    if *known != returned {
                        *known = None;
                    }
                }
            }
        }
    }
    let fields = combined.filter(|fields| returns > 0 && fields.iter().any(Option::is_some))?;
    Some(Contract {
        ty: func.sig.result,
        fields,
    })
}

/// Expands copies fed directly by contracted calls. Returns the number of forwarded fields.
fn expand_copies(
    func: &mut Func,
    interns: &Interns,
    contracts: &BTreeMap<JsName, Contract>,
) -> u32 {
    let mut calls: BTreeMap<ValueId, (usize, usize, Contract, Vec<ValueId>)> = BTreeMap::new();
    for (block_index, block) in func.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let (
                Some(result),
                OpKind::Call {
                    callee: Callee::Direct { link_name, .. },
                    args,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                    ..
                },
            ) = (op.result, &op.kind)
            else {
                continue;
            };
            let Some(name) = interns.str(*link_name) else {
                continue;
            };
            let Some(contract) = contracts.get(name) else {
                continue;
            };
            if contract.ty == func.values[result.0 as usize].ty
                && contract
                    .fields
                    .iter()
                    .flatten()
                    .all(|parameter| *parameter < args.len())
            {
                calls.insert(
                    result,
                    (block_index, op_index, contract.clone(), args.clone()),
                );
            }
        }
    }
    if calls.is_empty() {
        return 0;
    }

    let cfg = Cfg::of(func);
    // A source result may cross the structuralizer's block split before MIR's local copy. Admit
    // exactly the dominance-trivial form: the call is the source block's final op, that block
    // unconditionally enters a unique-predecessor successor, and the copy occurs before any use
    // of the result or effect in that successor. The same predicate within one block admits pure
    // producer bookkeeping while continuing to reject the mutation regression protected below.
    let mut safe_copies: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (block_index, block) in func.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let (Some(_), OpKind::Copy(source)) = (op.result, &op.kind) else {
                continue;
            };
            let Some((call_block, call_index, _, _)) = calls.get(source) else {
                continue;
            };
            let prefix_is_safe = |ops: &[Op]| {
                ops.iter().all(|between| {
                    if op_purity(&between.kind) == Purity::Effect {
                        return false;
                    }
                    let mut uses_source = false;
                    between.kind.for_each_use(|v| {
                        if v == *source {
                            uses_source = true;
                        }
                    });
                    !uses_source
                })
            };
            let safe = if *call_block == block_index {
                *call_index < op_index && prefix_is_safe(&block.ops[call_index + 1..op_index])
            } else {
                let source_block = &func.blocks[*call_block];
                *call_index + 1 == source_block.ops.len()
                    && matches!(
                        source_block.term,
                        crate::jir::repr::RTerm::Goto(target, _) if target == block.id
                    )
                    && cfg.preds[block_index].as_slice() == [RBlockId(*call_block as u32)]
                    && prefix_is_safe(&block.ops[..op_index])
            };
            if safe {
                safe_copies.insert((block_index, op_index));
            }
        }
    }

    let mut forwarded = 0u32;
    for block_index in 0..func.blocks.len() {
        let ops = std::mem::take(&mut func.blocks[block_index].ops);
        let mut rewritten = Vec::with_capacity(ops.len());
        for (op_index, mut op) in ops.into_iter().enumerate() {
            let (Some(result), OpKind::Copy(source)) = (op.result, &op.kind) else {
                rewritten.push(op);
                continue;
            };
            let Some((_call_block, _call_index, contract, args)) = calls.get(source) else {
                rewritten.push(op);
                continue;
            };
            if !safe_copies.contains(&(block_index, op_index)) {
                rewritten.push(op);
                continue;
            }
            let result_def = func.values[result.0 as usize];
            if result_def.ty != contract.ty || result_def.carrier != Carrier::Baseline {
                rewritten.push(op);
                continue;
            }
            let Some(shape) = candidates::shape(&interns.types, result_def.ty, 0) else {
                rewritten.push(op);
                continue;
            };
            if shape.len() != contract.fields.len()
                || !candidates::field_reads_resolve(&interns.types, result_def.ty, 0, shape.len())
                || shape.iter().zip(&contract.fields).any(|(&ty, parameter)| {
                    parameter
                        .is_some_and(|parameter| func.values[args[parameter].0 as usize].ty != ty)
                })
            {
                rewritten.push(op);
                continue;
            }

            let mut fields = Vec::with_capacity(shape.len());
            let mut prefix = Vec::with_capacity(shape.len() * 2);
            for (field, (&ty, parameter)) in shape.iter().zip(&contract.fields).enumerate() {
                let value = match parameter {
                    Some(parameter) => {
                        forwarded += 1;
                        args[*parameter]
                    }
                    None => {
                        let read = func.push_value(ty, Carrier::Baseline, op.origin);
                        prefix.push(Op {
                            result: Some(read),
                            origin: op.origin,
                            kind: OpKind::Field {
                                base: *source,
                                variant: None,
                                field: field as u32,
                            },
                        });
                        read
                    }
                };
                // Preserve the aggregate `Copy`'s per-field clone semantics. This is free for a
                // scalar and makes a fresh pointer/product value where the baseline requires one.
                let copied = func.push_value(ty, Carrier::Baseline, op.origin);
                prefix.push(Op {
                    result: Some(copied),
                    origin: op.origin,
                    kind: OpKind::Copy(value),
                });
                fields.push(copied);
            }
            rewritten.extend(prefix);
            op.kind = OpKind::Aggregate {
                ty: result_def.ty,
                variant: 0,
                active_field: None,
                fields,
            };
            rewritten.push(op);
        }
        func.blocks[block_index].ops = rewritten;
    }
    forwarded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::repr::{CarrierPolicy, FuncBuilder, RTerm};
    use crate::jir::semantic::{AbiSignature, ConstTree, DefinitionId, ItemId, Signature};
    use crate::jir::types::{
        FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor,
        TypeKind, TypeMeta,
    };
    use crate::jir::OriginId;

    const I32: TypeId = TypeId(1);
    const PAIR: TypeId = TypeId(2);
    const O: OriginId = OriginId(0);

    fn item(local_hash: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash,
            },
            instantiation: 0,
        }
    }

    fn interns() -> Interns {
        let mut interns = Interns::new();
        let scalar = JsFacts {
            int: Some(IntFacts {
                signed: true,
                bits: 32,
                repr: IntRepr::Number,
            }),
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: I32,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        };
        interns.intern_type(TypeDescriptor {
            id: I32,
            name: None,
            kind: TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: scalar,
        });
        let fields: Vec<FieldDescriptor> = (0..2)
            .map(|index| FieldDescriptor {
                key: FieldKey::Index(index),
                ty: I32,
                offset: u64::from(index) * 4,
            })
            .collect();
        interns.intern_type(TypeDescriptor {
            id: PAIR,
            name: None,
            kind: TypeKind::Tuple(vec![I32, I32]),
            layout: LayoutFacts {
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: None,
                is_indirect: true,
                needs_clone: true,
                crosses_transparent_union: false,
                transparent: None,
                peeled: PAIR,
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields,
            },
        });
        interns
    }

    fn signature(arguments: Vec<TypeId>, result: TypeId) -> Signature {
        Signature {
            abi: AbiSignature {
                parameters: arguments.len() as u16,
                ..AbiSignature::default()
            },
            arguments,
            result,
        }
    }

    #[test]
    fn a_copied_call_result_exposes_a_forwarded_parameter() {
        let mut interns = interns();
        let callee_name = JsName::new("callee".to_owned());
        let link_name = interns.intern_str(callee_name.as_str());

        let mut callee = FuncBuilder::new(item(1), signature(vec![I32, I32], PAIR));
        let entry = FuncBuilder::ENTRY;
        let first = callee.param(I32, O);
        let _second = callee.param(I32, O);
        let seven = callee.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 7, bytes: 4 }),
            I32,
            O,
        );
        let pair = callee.op(
            entry,
            OpKind::Aggregate {
                ty: PAIR,
                variant: 0,
                active_field: None,
                fields: vec![first, seven],
            },
            PAIR,
            O,
        );
        callee.terminate(entry, RTerm::Return(Some(pair)));
        let callee = callee.finish();
        let contract = forwarded_fields(&callee, &interns.types).expect("forwarding contract");
        assert_eq!(contract.fields, vec![Some(0), None]);

        let mut caller = FuncBuilder::new(item(2), signature(vec![I32, I32], PAIR));
        let first_arg = caller.param(I32, O);
        let second_arg = caller.param(I32, O);
        let copy_block = caller.block();
        let called = caller.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(1),
                    link_name,
                },
                function: None,
                args: vec![first_arg, second_arg],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            PAIR,
            O,
        );
        caller.terminate(entry, RTerm::Goto(copy_block, Vec::new()));
        // The SSA builder may split the MIR copy into the call's unique successor, with producer
        // bookkeeping before it. Neither should hide the forwarding contract.
        let _gap = caller.op(
            copy_block,
            OpKind::Const(ConstTree::Scalar { bits: 9, bytes: 4 }),
            I32,
            O,
        );
        let copied = caller.op(copy_block, OpKind::Copy(called), PAIR, O);
        caller.terminate(copy_block, RTerm::Return(Some(copied)));
        let mut caller = caller.finish();
        let contracts = BTreeMap::from([(callee_name, contract)]);

        assert_eq!(expand_copies(&mut caller, &interns, &contracts), 1);
        assert!(matches!(
            caller.blocks[copy_block.0 as usize]
                .ops
                .last()
                .map(|op| &op.kind),
            Some(OpKind::Aggregate { fields, .. }) if fields.len() == 2
        ));
        assert!(caller.blocks[copy_block.0 as usize]
            .ops
            .iter()
            .any(|op| matches!(op.kind, OpKind::Copy(value) if value == first_arg)));
        caller
            .verify_carriers(&interns.types, CarrierPolicy::BASELINE)
            .unwrap();
    }

    #[test]
    fn a_copy_after_mutation_does_not_forward_the_original_argument() {
        let mut interns = interns();
        let callee_name = JsName::new("callee".to_owned());
        let link_name = interns.intern_str(callee_name.as_str());

        let mut callee = FuncBuilder::new(item(1), signature(vec![I32, I32], PAIR));
        let entry = FuncBuilder::ENTRY;
        let first = callee.param(I32, O);
        let second = callee.param(I32, O);
        let pair = callee.op(
            entry,
            OpKind::Aggregate {
                ty: PAIR,
                variant: 0,
                active_field: None,
                fields: vec![first, second],
            },
            PAIR,
            O,
        );
        callee.terminate(entry, RTerm::Return(Some(pair)));
        let contract =
            forwarded_fields(&callee.finish(), &interns.types).expect("forwarding contract");

        let mut caller = FuncBuilder::new(item(2), signature(vec![I32, I32], PAIR));
        let first_arg = caller.param(I32, O);
        let second_arg = caller.param(I32, O);
        let called = caller.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(1),
                    link_name,
                },
                function: None,
                args: vec![first_arg, second_arg],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            PAIR,
            O,
        );
        caller.effect(
            entry,
            OpKind::SetField {
                base: called,
                variant: None,
                field: 0,
                v: second_arg,
            },
            O,
        );
        let copied = caller.op(entry, OpKind::Copy(called), PAIR, O);
        caller.terminate(entry, RTerm::Return(Some(copied)));
        let mut caller = caller.finish();
        let original = caller.clone();
        let contracts = BTreeMap::from([(callee_name, contract)]);

        assert_eq!(expand_copies(&mut caller, &interns, &contracts), 0);
        assert_eq!(caller, original, "the delayed copy remains intact");
    }
}
