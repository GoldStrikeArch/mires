//! Forwards immutable indexed reads through locally built slice views.
//!
//! `SliceView(ptr, len)` is a logical fat pointer. When an immutable indexed borrow is its only
//! consumer, building `{ buf, off, len }` first is unnecessary: offset the element pointer and
//! borrow through it directly. The original view remains whenever any other use needs it, so this
//! pass changes no storage or aliasing behavior.

use std::collections::BTreeMap;

use crate::jir::repr::{Dominators, Func, Op, OpKind, Proj, Root, ValueDef, ValueId};
use crate::jir::semantic::BinaryOp;
use crate::jir::TypeTable;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct View {
    ptr: ValueId,
    len: ValueId,
}

/// Rewrites indexed borrows and returns their count.
pub fn run(func: &mut Func, table: &TypeTable) -> u32 {
    let incoming = incoming(func);
    let doms = Dominators::of(func);
    let mut views: BTreeMap<ValueId, View> = BTreeMap::new();

    // Copies, whole-view reborrows, and block parameters can form short chains. A small monotone
    // fixpoint discovers them all without changing the function.
    for _ in 0..4 {
        let before = views.len();
        for &block in &doms.rpo {
            if block.0 != 0 {
                for (position, param) in func.blocks[block.0 as usize].params.iter().enumerate() {
                    let mut agreed: Option<View> = None;
                    let mut valid = true;
                    for args in &incoming[block.0 as usize] {
                        let Some(arg) = args.get(position) else {
                            valid = false;
                            break;
                        };
                        if arg == param {
                            continue;
                        }
                        let Some(view) = views.get(arg).copied() else {
                            valid = false;
                            break;
                        };
                        match agreed {
                            None => agreed = Some(view),
                            Some(known) if known == view => {}
                            Some(_) => {
                                valid = false;
                                break;
                            }
                        }
                    }
                    if valid {
                        if let Some(view) = agreed {
                            views.insert(*param, view);
                        }
                    }
                }
            }
            for op in &func.blocks[block.0 as usize].ops {
                let Some(result) = op.result else {
                    continue;
                };
                let view = match &op.kind {
                    OpKind::SliceView { ptr, len }
                        if !super::range::is_slice_pointer(
                            table,
                            func.values[ptr.0 as usize].ty,
                        ) =>
                    {
                        Some(View {
                            ptr: *ptr,
                            len: *len,
                        })
                    }
                    OpKind::Copy(value) | OpKind::Move(value) => views.get(value).copied(),
                    OpKind::BorrowView {
                        root: Root::Value(value),
                        path,
                        mutable: false,
                    } if matches!(path.as_slice(), [Proj::Deref])
                        && super::range::is_slice_pointer(
                            table,
                            func.values[result.0 as usize].ty,
                        ) =>
                    {
                        views.get(value).copied()
                    }
                    _ => None,
                };
                if let Some(view) = view {
                    views.insert(result, view);
                }
            }
        }
        if views.len() == before {
            break;
        }
    }
    if views.is_empty() {
        return 0;
    }

    let mut changed = 0u32;
    for block_index in 0..func.blocks.len() {
        let ops = std::mem::take(&mut func.blocks[block_index].ops);
        let mut rewritten = Vec::with_capacity(ops.len());
        for mut op in ops {
            let view_and_index = match &op.kind {
                OpKind::BorrowView {
                    root: Root::Value(value),
                    path,
                    mutable: false,
                } => match path.as_slice() {
                    [Proj::Deref, Proj::Index(index)] => {
                        views.get(value).copied().map(|view| (view, *index))
                    }
                    _ => None,
                },
                _ => None,
            };
            let Some((view, index)) = view_and_index else {
                rewritten.push(op);
                continue;
            };
            let ptr_def = func.values[view.ptr.0 as usize];
            let offset = ValueId(func.values.len() as u32);
            func.values.push(ValueDef {
                ty: ptr_def.ty,
                carrier: ptr_def.carrier,
                origin: op.origin,
            });
            rewritten.push(Op {
                result: Some(offset),
                origin: op.origin,
                kind: OpKind::Binary {
                    op: BinaryOp::Offset,
                    l: view.ptr,
                    r: index,
                    ty: ptr_def.ty,
                },
            });
            op.kind = OpKind::BorrowView {
                root: Root::Value(offset),
                path: vec![Proj::Deref],
                mutable: false,
            };
            rewritten.push(op);
            changed += 1;
        }
        func.blocks[block_index].ops = rewritten;
    }
    changed
}

fn incoming(func: &Func) -> Vec<Vec<Vec<ValueId>>> {
    let mut incoming = vec![Vec::new(); func.blocks.len()];
    for block in &func.blocks {
        for (target, args) in block.term.edges() {
            incoming[target.0 as usize].push(args.to_vec());
        }
    }
    incoming
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::repr::{CarrierPolicy, FuncBuilder, RTerm};
    use crate::jir::semantic::{AbiSignature, ConstTree, DefinitionId, ItemId, Signature, TypeId};
    use crate::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, PointerForm, TypeDescriptor, TypeKind, TypeMeta,
    };
    use crate::jir::OriginId;

    const U32: TypeId = TypeId(1);
    const U8: TypeId = TypeId(2);
    const SLICE: TypeId = TypeId(3);
    const RAW_U8: TypeId = TypeId(4);
    const REF_SLICE: TypeId = TypeId(5);
    const REF_U8: TypeId = TypeId(6);
    const O: OriginId = OriginId(0);

    fn js(peeled: TypeId) -> JsFacts {
        JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        }
    }

    fn table() -> TypeTable {
        let mut table = TypeTable::new();
        for (id, bits) in [(U32, 32), (U8, 8)] {
            let mut facts = js(id);
            facts.int = Some(IntFacts {
                signed: false,
                bits,
                repr: IntRepr::Number,
            });
            facts.byte_element = bits == 8;
            table.insert(TypeDescriptor {
                id,
                name: None,
                kind: TypeKind::Integer {
                    signed: false,
                    bits,
                },
                layout: LayoutFacts {
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: facts,
            });
        }
        table.insert(TypeDescriptor {
            id: SLICE,
            name: None,
            kind: TypeKind::Slice(U8),
            layout: LayoutFacts::default(),
            meta: TypeMeta::default(),
            js: js(SLICE),
        });
        for (id, pointee, form, raw) in [
            (RAW_U8, U8, PointerForm::Slot, true),
            (REF_SLICE, SLICE, PointerForm::FatSlice, false),
            (REF_U8, U8, PointerForm::Slot, false),
        ] {
            let mut facts = js(id);
            facts.pointer_form = Some(form);
            table.insert(TypeDescriptor {
                id,
                name: None,
                kind: if raw {
                    TypeKind::RawPointer {
                        pointee,
                        mutable: false,
                    }
                } else {
                    TypeKind::Reference {
                        pointee,
                        mutable: false,
                    }
                },
                layout: LayoutFacts {
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: facts,
            });
        }
        table
    }

    #[test]
    fn an_immutable_index_borrows_through_the_source_pointer() {
        let mut b = FuncBuilder::new(
            ItemId {
                definition: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                instantiation: 0,
            },
            Signature {
                abi: AbiSignature {
                    parameters: 2,
                    ..AbiSignature::default()
                },
                arguments: vec![RAW_U8, U32],
                result: U8,
            },
        );
        let entry = FuncBuilder::ENTRY;
        let ptr = b.param(RAW_U8, O);
        let index = b.param(U32, O);
        let len = b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits: 16, bytes: 4 }),
            U32,
            O,
        );
        let view = b.op(entry, OpKind::SliceView { ptr, len }, REF_SLICE, O);
        let borrowed = b.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(view),
                path: vec![Proj::Deref, Proj::Index(index)],
                mutable: false,
            },
            REF_U8,
            O,
        );
        let read = b.op(entry, OpKind::Deref(borrowed), U8, O);
        b.terminate(entry, RTerm::Return(Some(read)));
        let mut func = b.finish();

        assert_eq!(run(&mut func, &table()), 1);
        assert!(func.blocks[0].ops.iter().any(|op| matches!(
            op.kind,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l,
                r,
                ..
            } if l == ptr && r == index
        )));
        assert!(func.blocks[0].ops.iter().any(|op| matches!(
            &op.kind,
            OpKind::BorrowView {
                root: Root::Value(_),
                path,
                mutable: false,
            } if path.as_slice() == [Proj::Deref]
        )));
        func.verify_carriers(&table(), CarrierPolicy::BASELINE)
            .unwrap();
    }
}
