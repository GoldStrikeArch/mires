//! Devirtualization, narrow by design (phase 5, D6): a dispatch whose target is **unique and
//! constant** becomes a direct call; everything else is left exactly as it was.
//!
//! Two shapes, both syntactic:
//!
//! * a [`Callee::Indirect`] call whose function operand traces through `Copy`/`Move` to a constant
//!   function reference ([`ConstTree::Global`] at offset 0) — the fn-pointer constant;
//! * a [`Callee::Virtual`] call whose receiver traces to the unsizing [`OpKind::Cast`] of a plain
//!   reference, so the vtable is the compile-time constant of one concrete type: the
//!   [`VtableEntry::Method`] at the call's slot is the unique target, and the receiver argument
//!   becomes the cast's thin operand (the virtual lowering passes the data half; the method body
//!   takes the same reference).
//!
//! Bounded-set dispatch groups are not built (D6: YAGNI — the idiomatic sidecar shows four
//! `unknown-call` candidates). A rewrite happens only when the target's body is one the lane
//! also built, with a positionally aligned signature and no trailing caller `Location`, so the
//! direct lowering spells exactly the call the dispatch would have made. Every rewrite is applied
//! transactionally and re-verified under its observed carriers; any failure reverts the whole
//! function.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::interns::{Interns, StrId};
use crate::jir::repr::{Func, OpKind};
use crate::jir::semantic::{Callee, ConstTree, GlobalReference, ItemId, TypeId, VtableEntry};
use crate::jir::types::TypeKind;
use crate::jir::BodyKind;
use crate::lower::js::unsize::VtableIndex;

use super::cfg::{Cfg, DefUse};
use super::tx::{try_rewrite_func, TxOutcome};

/// What the pass did, for the sidecar's `devirt` block.
#[derive(Clone, Debug, Default)]
pub struct DevirtStats {
    /// Call sites rewritten to direct calls.
    pub applied: u32,
    pub witnesses_recorded: u32,
    pub witnesses_failed: u32,
    /// The functions that changed.
    pub items: BTreeSet<JsName>,
}

/// One planned rewrite: the site and the direct callee that replaces its dispatch.
struct Rewrite {
    block: usize,
    op: usize,
    item: ItemId,
    link_name: StrId,
    /// `Some` for a virtual site: the thin receiver that replaces the fat-pointer argument.
    receiver: Option<crate::jir::repr::ValueId>,
}

/// Runs the pass over every `Ok` func, in place.
pub fn run(
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &Interns>,
    items: &BTreeMap<JsName, LinkItem>,
    vtables: &VtableIndex,
) -> DevirtStats {
    let mut stats = DevirtStats::default();
    let names: Vec<JsName> = funcs.keys().cloned().collect();
    for name in names {
        let Some(caller_interns) = interns.get(&name).copied() else {
            continue;
        };
        let Some(Ok(func)) = funcs.get(&name) else {
            continue;
        };
        let rewrites = plan(func, caller_interns, funcs, items, vtables);
        if rewrites.is_empty() {
            continue;
        }
        let table = &caller_interns.types;
        match try_rewrite_func(
            func,
            table,
            |rewritten| {
                let mut applied_count = 0u32;
                for rewrite in &rewrites {
                    let op = &mut rewritten.blocks[rewrite.block].ops[rewrite.op];
                    let OpKind::Call {
                        callee,
                        function,
                        args,
                        ..
                    } = &mut op.kind
                    else {
                        continue;
                    };
                    *callee = Callee::Direct {
                        item: rewrite.item,
                        link_name: rewrite.link_name,
                    };
                    *function = None;
                    if let Some(receiver) = rewrite.receiver {
                        args[0] = receiver;
                    }
                    applied_count += 1;
                }
                (applied_count > 0).then_some(applied_count)
            },
            |_, _| true,
        ) {
            TxOutcome::Unchanged => {}
            TxOutcome::RolledBack => {
                stats.witnesses_failed += 1;
            }
            TxOutcome::Committed {
                func: rewritten,
                payload: applied_count,
            } => {
                stats.applied += applied_count;
                stats.witnesses_recorded += applied_count;
                stats.items.insert(name.clone());
                funcs.insert(name, Ok(rewritten));
            }
        }
    }
    stats
}

/// Per-function context shared while planning devirtualization rewrites.
struct PlanCx<'a> {
    func: &'a Func,
    caller_interns: &'a Interns,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    items: &'a BTreeMap<JsName, LinkItem>,
    vtables: &'a VtableIndex,
    def_use: DefUse,
    forwarded: BTreeMap<u32, crate::jir::repr::ValueId>,
}

impl<'a> PlanCx<'a> {
    fn new(
        func: &'a Func,
        caller_interns: &'a Interns,
        funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
        items: &'a BTreeMap<JsName, LinkItem>,
        vtables: &'a VtableIndex,
    ) -> Self {
        // Every value's defining op, for the constant traces, plus the one incoming argument of
        // each single-predecessor block's parameters (an inlined body's entry takes that shape,
        // and the constant a caller passed flows through it).
        let def_use = DefUse::of(func);
        let incoming = Cfg::incoming_args(func);
        let mut forwarded: BTreeMap<u32, crate::jir::repr::ValueId> = BTreeMap::new();
        for block in &func.blocks {
            let [args] = incoming[block.id.0 as usize].as_slice() else {
                continue;
            };
            for (position, param) in block.params.iter().enumerate() {
                if let Some(arg) = args.get(position) {
                    forwarded.insert(param.0, *arg);
                }
            }
        }
        Self {
            func,
            caller_interns,
            funcs,
            items,
            vtables,
            def_use,
            forwarded,
        }
    }

    fn resolve(&self, mut v: crate::jir::repr::ValueId) -> Option<&'a OpKind> {
        for _ in 0..64 {
            if let Some(def) = self.def_use.def_kind(self.func, v) {
                match def {
                    OpKind::Copy(inner) | OpKind::Move(inner) => v = *inner,
                    other => return Some(other),
                }
            } else {
                v = *self.forwarded.get(&v.0)?;
            }
        }
        None
    }

    /// A reified fn pointer's identity: a `ReifyFnPointer` coercion carries it in the `FnDef`
    /// type's resolved instance (the operand is zero-sized), a decoded constant in a `Global`
    /// function reference; the coercion casts between pointer shapes are peeled. The virtual trace
    /// below must NOT peel the unsizing cast — it IS its evidence — so only this helper walks
    /// casts.
    fn fn_target(&self, mut v: crate::jir::repr::ValueId) -> Option<(ItemId, StrId)> {
        use crate::jir::semantic::CastKind;
        for _ in 0..64 {
            match self.resolve(v)? {
                OpKind::Const(ConstTree::Global {
                    target: GlobalReference::Function { item, link_name },
                    offset: 0,
                }) => return Some((*item, *link_name)),
                OpKind::Cast {
                    kind: CastKind::PointerCoercion,
                    v: inner,
                    from,
                    ..
                } => {
                    if let Some(TypeKind::FnDef {
                        instance: Some(instance),
                        ..
                    }) = self.caller_interns.types.get(*from).map(|d| &d.kind)
                    {
                        return Some((instance.item, instance.link_name));
                    }
                    v = *inner;
                }
                OpKind::Cast {
                    kind: CastKind::FnPtrToPtr | CastKind::Subtype,
                    v: inner,
                    ..
                } => v = *inner,
                _ => return None,
            }
        }
        None
    }

    /// Whether the target named `link_name` (a caller-scoped id) is a body the lane built whose
    /// parameters align positionally with `args` arguments and that takes no trailing `Location`.
    fn aligned(&self, link_name: StrId, args: usize) -> Option<()> {
        let name = self.caller_interns.str(link_name)?;
        let target = self.funcs.get(name)?.as_ref().ok()?;
        (!target.sig.abi.caller_location
            && target.sig.abi.spread.is_none()
            && target.params.len() == args)
            .then_some(())
    }

    /// The unique target of a virtual call whose receiver is the unsizing cast of a plain
    /// reference, plus the thin receiver value that replaces the fat pointer.
    fn virtual_target(
        &self,
        args: &[crate::jir::repr::ValueId],
        slot: u32,
        receiver_ty: TypeId,
    ) -> Option<(ItemId, StrId, crate::jir::repr::ValueId)> {
        let table = &self.caller_interns.types;
        let fat = *args.first()?;
        let OpKind::Cast {
            kind: crate::jir::semantic::CastKind::PointerCoercion,
            v,
            from,
            ..
        } = self.resolve(fat)?
        else {
            return None;
        };
        // A *reference* to a concrete pointee: the virtual lowering hands the vtable's method the
        // data half of the fat pointer, which for a reference is the operand unchanged. A raw
        // pointer is re-wrapped at the cast (`raw_to_ref`) and is not matched.
        let TypeKind::Reference {
            pointee,
            mutable: _,
        } = table.get(*from)?.kind
        else {
            return None;
        };
        if matches!(table.get(pointee)?.kind, TypeKind::Dynamic { .. }) {
            return None; // dyn-to-dyn: not a constant vtable.
        }
        let TypeKind::Dynamic { principal, .. } = table.get(receiver_ty)?.kind else {
            return None;
        };
        let vtable_name = [pointee, table.peeled(pointee)]
            .iter()
            .find_map(|key| self.vtables.get(&(*key, principal)))?;
        let vtable = self.items.get(vtable_name.as_str())?;
        let BodyKind::Vtable { entries, .. } = &vtable.jir.body.as_ref()?.kind else {
            return None;
        };
        let VtableEntry::Method(method) = entries.get(slot as usize)? else {
            return None;
        };
        // The method's name is interned in the vtable's object; the rewrite has to spell it with a
        // caller-scoped id, which exists only if the caller's object interned the same string.
        let target = vtable.jir.interns()?.str(method.link_name)?;
        let position = self
            .caller_interns
            .strings
            .iter()
            .position(|s| s == target)?;
        let link_name = StrId(position as u32);
        self.aligned(link_name, args.len())?;
        // The thin operand must be a value of this function (it is: the cast is one of its ops).
        debug_assert!((v.0 as usize) < self.func.values.len());
        Some((method.item, link_name, *v))
    }
}

/// The rewrites one function admits, in block-and-op order.
fn plan(
    func: &Func,
    caller_interns: &Interns,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    items: &BTreeMap<JsName, LinkItem>,
    vtables: &VtableIndex,
) -> Vec<Rewrite> {
    let cx = PlanCx::new(func, caller_interns, funcs, items, vtables);
    let mut rewrites = Vec::new();
    for (block_index, block) in func.blocks.iter().enumerate() {
        for (op_index, op) in block.ops.iter().enumerate() {
            let OpKind::Call {
                callee,
                function,
                args,
                track_caller: None,
                host: None,
                ..
            } = &op.kind
            else {
                continue;
            };
            let planned = match callee {
                Callee::Indirect { .. } => function.and_then(|f| {
                    cx.fn_target(f).and_then(|(item, link_name)| {
                        cx.aligned(link_name, args.len())?;
                        Some(Rewrite {
                            block: block_index,
                            op: op_index,
                            item,
                            link_name,
                            receiver: None,
                        })
                    })
                }),
                Callee::Virtual { slot, receiver } => cx
                    .virtual_target(args, *slot, *receiver)
                    .map(|(item, link_name, thin)| Rewrite {
                        block: block_index,
                        op: op_index,
                        item,
                        link_name,
                        receiver: Some(thin),
                    }),
                _ => None,
            };
            rewrites.extend(planned);
        }
    }
    rewrites
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::repr::{FuncBuilder, RTerm};
    use crate::jir::semantic::{AbiSignature, CallSemantic, DefinitionId, Signature};
    use crate::jir::types::{
        IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeKind, TypeMeta,
    };
    use crate::jir::OriginId;

    const O: OriginId = OriginId(0);
    const I32: TypeId = TypeId(1);

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash: n,
            },
            instantiation: 0,
        }
    }

    fn sig(args: usize) -> Signature {
        Signature {
            arguments: vec![I32; args],
            result: I32,
            abi: AbiSignature {
                parameters: args as u16,
                ..AbiSignature::default()
            },
        }
    }

    fn interns() -> Interns {
        let mut interns = Interns::new();
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
            js: JsFacts {
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
            },
        });
        interns
    }

    /// `fn g(x) { return x; }`
    fn identity() -> Func {
        let mut b = FuncBuilder::new(item(2), sig(1));
        let x = b.param(I32, O);
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(x)));
        b.finish()
    }

    /// `fn f(p) { return (CONST g)(p); }` — or, with `constant` false, through its own
    /// parameter, whose target no trace can name.
    fn calls_through_pointer(g: StrId, constant: bool) -> Func {
        let mut b = FuncBuilder::new(item(1), sig(2));
        let p = b.param(I32, O);
        let fnptr = b.param(I32, O);
        let function = if constant {
            b.op(
                FuncBuilder::ENTRY,
                OpKind::Const(ConstTree::Global {
                    target: GlobalReference::Function {
                        item: item(2),
                        link_name: g,
                    },
                    offset: 0,
                }),
                I32,
                O,
            )
        } else {
            fnptr
        };
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Indirect { signature: I32 },
                function: Some(function),
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        b.finish()
    }

    fn setup(constant: bool) -> (Interns, BTreeMap<JsName, Result<Func, Decline>>) {
        let mut interns = interns();
        let g = interns.intern_str("g");
        let mut funcs: BTreeMap<JsName, Result<Func, Decline>> = BTreeMap::new();
        funcs.insert(JsName::new("g".to_owned()), Ok(identity()));
        funcs.insert(
            JsName::new("f".to_owned()),
            Ok(calls_through_pointer(g, constant)),
        );
        (interns, funcs)
    }

    fn devirt(
        interns: &Interns,
        funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ) -> DevirtStats {
        let interns_map: BTreeMap<JsName, &Interns> =
            funcs.keys().map(|name| (name.clone(), interns)).collect();
        let items = BTreeMap::new();
        let vtables = VtableIndex::new();
        run(funcs, &interns_map, &items, &vtables)
    }

    #[test]
    fn a_constant_fn_pointer_call_becomes_direct() {
        let (interns, mut funcs) = setup(true);
        let stats = devirt(&interns, &mut funcs);
        assert_eq!(stats.applied, 1);
        assert_eq!(stats.witnesses_failed, 0);
        let f = funcs
            .get(&JsName::new("f".to_owned()))
            .and_then(|f| f.as_ref().ok())
            .expect("f");
        let g = interns.strings.iter().position(|s| s == "g").unwrap() as u32;
        let direct = f.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            matches!(
                &op.kind,
                OpKind::Call {
                    callee: Callee::Direct { link_name, .. },
                    function: None,
                    ..
                } if link_name.0 == g
            )
        });
        assert!(direct, "the dispatch was rewritten to a direct call");
    }

    #[test]
    fn a_non_unique_target_is_left_alone() {
        let (interns, mut funcs) = setup(false);
        let before = funcs.clone();
        let stats = devirt(&interns, &mut funcs);
        assert_eq!(stats.applied, 0);
        assert_eq!(
            funcs.get(&JsName::new("f".to_owned())),
            before.get(&JsName::new("f".to_owned())),
            "an unknown fn pointer keeps its indirect call"
        );
    }

    #[test]
    fn a_misaligned_target_is_left_alone() {
        // `g` takes one argument; the call passes two: the rewrite must not happen.
        let mut interns = interns();
        let g = interns.intern_str("g");
        let mut b = FuncBuilder::new(item(1), sig(3));
        let p = b.param(I32, O);
        let q = b.param(I32, O);
        let _ = b.param(I32, O);
        let function = b.op(
            FuncBuilder::ENTRY,
            OpKind::Const(ConstTree::Global {
                target: GlobalReference::Function {
                    item: item(2),
                    link_name: g,
                },
                offset: 0,
            }),
            I32,
            O,
        );
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Indirect { signature: I32 },
                function: Some(function),
                args: vec![p, q],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            I32,
            O,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        let mut funcs: BTreeMap<JsName, Result<Func, Decline>> = BTreeMap::new();
        funcs.insert(JsName::new("g".to_owned()), Ok(identity()));
        funcs.insert(JsName::new("f".to_owned()), Ok(b.finish()));
        let stats = devirt(&interns, &mut funcs);
        assert_eq!(stats.applied, 0);
    }
}
