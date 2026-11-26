use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::param_use::ParamUse;
use crate::analysis::refs::{self, RefScan};
use crate::item::{ItemKind, ItemMap, JsName, LinkItem, Linkage};
use crate::jir::decline::{BoundaryDecline, Decline};
use crate::jir::repr::{Func, Op, OpKind, RTerm, Root, ValueId};
use crate::jir::semantic::{BodySemantic, Callee, ConstTree, ItemId, TypeId};
use crate::jir::summary::ItemClass;
use crate::jir::types::{EnumRepr, NominalForm, PinnedBoundary, PointerForm, TypeKind, TypeTable};
use crate::opts::Opts;

use super::plan::{
    allocate_spill_names, close_over_forwards, field_only_value, inherit_clone_retargets, refuses,
    result_forwards_survive, tagged_chain_members, terminal_reason, Candidate,
};
use super::rewrite::{rewrite_callee_with, unbox_scalar_ref_cells};
use super::shapes::{
    flat_param, flat_param_with, flat_result, product_node, recursive_shape, scalar_leaf_result,
    shapes_agree, store_view_ty,
};
use super::*;
use crate::jir::interns::StrId;
use crate::jir::repr::{FuncBuilder, Proj};
use crate::jir::semantic::{
    AbiSignature, CastKind, DefinitionId, DirectItemRef, GlobalReference, HostPayload, LocalId,
    Signature, SpreadArgument,
};
use crate::jir::types::{
    FieldDescriptor, FieldKey, JsFacts, LayoutFacts, TypeDescriptor, TypeMeta,
};
use crate::jir::{Interns, OriginId};

const U32: TypeId = TypeId(1);
const PAIR: TypeId = TypeId(2);
const REF_PAIR: TypeId = TypeId(3);
const UNIT: TypeId = TypeId(4);
const FN_DEF: TypeId = TypeId(5);
/// `Result<u32, u32>`: two variants, so its fields depend on the value.
const RESULT: TypeId = TypeId(6);
/// `(u32, ())`: one field that says nothing.
const CTX: TypeId = TypeId(7);
const REF_U32: TypeId = TypeId(8);
const NO_ORIGIN: OriginId = OriginId(0);

fn item(local_hash: u64) -> ItemId {
    ItemId {
        definition: DefinitionId {
            crate_id: 0,
            local_hash,
        },
        instantiation: u128::from(local_hash),
    }
}

fn descriptor(id: TypeId, kind: TypeKind) -> TypeDescriptor {
    let zst = id == UNIT || id == FN_DEF;
    let aggregate = matches!(id, PAIR | RESULT | CTX);
    TypeDescriptor {
        id,
        name: None,
        kind,
        layout: LayoutFacts {
            size: Some(if zst { 0 } else { 8 }),
            zst,
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int: None,
            is_indirect: aggregate,
            needs_clone: aggregate,
            crosses_transparent_union: false,
            transparent: None,
            peeled: id,
            unsized_tail: None,
            enum_repr: match id {
                RESULT => Some(crate::jir::types::EnumRepr::Tag { fieldless: false }),
                _ => None,
            },
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: match id {
                PAIR => (0..2)
                    .map(|index| FieldDescriptor {
                        key: FieldKey::Index(index),
                        ty: U32,
                        offset: u64::from(index) * 4,
                    })
                    .collect(),
                CTX => vec![
                    FieldDescriptor {
                        key: FieldKey::Index(0),
                        ty: U32,
                        offset: 0,
                    },
                    FieldDescriptor {
                        key: FieldKey::Index(1),
                        ty: UNIT,
                        offset: 4,
                    },
                ],
                _ => Vec::new(),
            },
        },
    }
}

/// `u32`, `(u32, u32)`, `&mut (u32, u32)`, `()`, and a `fn` item resolving to item 9.
fn interns() -> Interns {
    let mut interns = Interns::new();
    for descriptor in [
        descriptor(
            U32,
            TypeKind::Integer {
                signed: false,
                bits: 32,
            },
        ),
        descriptor(PAIR, TypeKind::Tuple(vec![U32, U32])),
        descriptor(
            REF_PAIR,
            TypeKind::Reference {
                pointee: PAIR,
                mutable: true,
            },
        ),
        descriptor(UNIT, TypeKind::Tuple(Vec::new())),
        descriptor(CTX, TypeKind::Tuple(vec![U32, UNIT])),
        descriptor(
            REF_U32,
            TypeKind::Reference {
                pointee: U32,
                mutable: true,
            },
        ),
        {
            let variant = |name: u32, ok: bool| crate::jir::types::VariantDescriptor {
                name: StrId(name),
                fields: vec![FieldDescriptor {
                    key: FieldKey::Index(0),
                    ty: U32,
                    offset: 4,
                }],
                logical_discr: i128::from(ok),
                physical_tag: None,
                result_ok: ok,
                saved_local_keys: Vec::new(),
            };
            descriptor(
                RESULT,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: 7,
                    },
                    args: vec![U32, U32],
                    form: NominalForm::Enum,
                    variants: vec![variant(0, true), variant(0, false)],
                    niche: None,
                },
            )
        },
        descriptor(
            FN_DEF,
            TypeKind::FnDef {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 9,
                },
                sig: crate::jir::interns::SigId(0),
                instance: Some(DirectItemRef {
                    item: item(9),
                    link_name: StrId(0),
                }),
            },
        ),
    ] {
        interns.types.insert(descriptor);
    }
    interns
}

fn signature(arguments: Vec<TypeId>, result: TypeId) -> Signature {
    let parameters = arguments.len() as u16;
    Signature {
        arguments,
        result,
        abi: AbiSignature {
            parameters,
            spread: None,
            caller_location: false,
            inline_never: false,
            inline_hint: false,
            inline_always: false,
        },
    }
}

/// `fn step(p: &mut (u32, u32)) { let d = *p; d.1 = d.0; }`
fn callee() -> Func {
    let mut builder = FuncBuilder::new(item(1), signature(vec![REF_PAIR], UNIT));
    let p = builder.param(REF_PAIR, NO_ORIGIN);
    let d = builder.op(FuncBuilder::ENTRY, OpKind::Deref(p), PAIR, NO_ORIGIN);
    let high = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Field {
            base: d,
            variant: None,
            field: 0,
        },
        U32,
        NO_ORIGIN,
    );
    builder.effect(
        FuncBuilder::ENTRY,
        OpKind::SetField {
            base: d,
            variant: None,
            field: 1,
            v: high,
        },
        NO_ORIGIN,
    );
    builder.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
    builder.finish()
}

/// `fn caller(x: u32) -> u32 { let mut p = (x, x); step(&mut p); p.0 }`
fn caller() -> Func {
    let mut builder = FuncBuilder::new(item(2), signature(vec![U32], U32));
    let x = builder.param(U32, NO_ORIGIN);
    let pair = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Aggregate {
            ty: PAIR,
            variant: 0,
            active_field: None,
            fields: vec![x, x],
        },
        PAIR,
        NO_ORIGIN,
    );
    let view = builder.op(
        FuncBuilder::ENTRY,
        OpKind::BorrowView {
            root: Root::Value(pair),
            path: Vec::new(),
            mutable: true,
        },
        REF_PAIR,
        NO_ORIGIN,
    );
    builder.effect(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(1),
                link_name: StrId(0),
            },
            function: None,
            args: vec![view],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        NO_ORIGIN,
    );
    let low = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Field {
            base: pair,
            variant: None,
            field: 0,
        },
        U32,
        NO_ORIGIN,
    );
    builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(low)));
    builder.finish()
}

/// `fn make_pair(x: u32) -> Pair { Pair(x, x) }`
fn result_callee() -> Func {
    let mut builder = FuncBuilder::new(item(3), signature(vec![U32], PAIR));
    let x = builder.param(U32, NO_ORIGIN);
    let pair = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Aggregate {
            ty: PAIR,
            variant: 0,
            active_field: None,
            fields: vec![x, x],
        },
        PAIR,
        NO_ORIGIN,
    );
    builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(pair)));
    builder.finish()
}

/// `fn use_pair(x: u32) -> u32 { make_pair(x).0 }`
fn result_caller() -> Func {
    let mut builder = FuncBuilder::new(item(4), signature(vec![U32], U32));
    let x = builder.param(U32, NO_ORIGIN);
    let pair = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(3),
                link_name: StrId(0),
            },
            function: None,
            args: vec![x],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        PAIR,
        NO_ORIGIN,
    );
    let left = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Field {
            base: pair,
            variant: None,
            field: 0,
        },
        U32,
        NO_ORIGIN,
    );
    builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(left)));
    builder.finish()
}

/// `fn forward_pair(x: u32) -> u32 { consume_pair(make_pair(x)) }`
fn forwarding_result_caller() -> Func {
    let mut builder = FuncBuilder::new(item(4), signature(vec![U32], U32));
    let x = builder.param(U32, NO_ORIGIN);
    let pair = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(3),
                link_name: StrId(0),
            },
            function: None,
            args: vec![x],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        PAIR,
        NO_ORIGIN,
    );
    let consumed = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(5),
                link_name: StrId(0),
            },
            function: None,
            args: vec![pair],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        U32,
        NO_ORIGIN,
    );
    builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(consumed)));
    builder.finish()
}

fn flat_pair() -> FlatParam {
    FlatParam::aggregate(
        PAIR,
        vec![U32, U32],
        vec![0, 1],
        ParamAccess::MutRef { written: true },
    )
}

fn flat_sig() -> FlatSig {
    FlatSig {
        params: vec![Some(flat_pair())],
        result: None,
        spill: 1,
        returns_out_field: true,
    }
}

#[test]
fn slice_positions_account_for_earlier_product_expansion() {
    let slice = FlatParam::slice(REF_PAIR, vec![REF_PAIR, U32, U32]);
    let sig = FlatSig {
        params: vec![Some(flat_pair()), Some(slice), None],
        result: None,
        spill: 0,
        returns_out_field: false,
    };
    assert_eq!(sig.slice_jir_positions(), vec![2]);
}

#[test]
fn a_private_clone_inherits_its_sources_direct_call_retargets() {
    let source = JsName::new("source".to_owned());
    let clone = JsName::new("source$n1".to_owned());
    let target = JsName::new("target$n1".to_owned());
    let mut plan = AbiPlan::default();
    plan.clones.insert(clone.clone(), source.clone());
    plan.call_retargets
        .entry(source)
        .or_default()
        .insert(item(2), target.clone());

    inherit_clone_retargets(&mut plan);

    assert_eq!(plan.call_retargets(&clone).get(&item(2)), Some(&target));
}

#[test]
fn economic_groups_couple_shared_callers_and_subset_the_executable_plan() {
    let one = JsName::new("one".to_owned());
    let two = JsName::new("two".to_owned());
    let three = JsName::new("three".to_owned());
    let shared = JsName::new("shared".to_owned());
    let separate = JsName::new("separate".to_owned());
    let mut plan = AbiPlan::default();
    for (id, name) in [
        (item(1), one.clone()),
        (item(2), two.clone()),
        (item(3), three.clone()),
    ] {
        plan.flat.insert(id, FlatSig::default());
        plan.names.insert(id, name.clone());
        plan.select(&name, &FlatSig::default());
    }
    plan.callers
        .insert(item(1), BTreeSet::from([shared.clone()]));
    plan.callers.insert(item(2), BTreeSet::from([shared]));
    plan.callers.insert(item(3), BTreeSet::from([separate]));

    assert_eq!(
        plan.economic_groups(),
        vec![
            BTreeSet::from([item(1), item(2)]),
            BTreeSet::from([item(3)])
        ]
    );
    let subset = plan.subset(&BTreeSet::from([item(1), item(2)]));
    assert_eq!(
        subset.flat.keys().copied().collect::<Vec<_>>(),
        vec![item(1), item(2)]
    );
    assert_eq!(subset.source_name(item(1)), Some(&one));
    assert!(!subset.touches(&three));
}

#[test]
fn economic_groups_close_transitively_across_a_later_bridge() {
    let mut plan = AbiPlan::default();
    for (id, name) in [(item(1), "one"), (item(2), "two"), (item(3), "three")] {
        let name = JsName::new(name.to_owned());
        plan.flat.insert(id, FlatSig::default());
        plan.names.insert(id, name.clone());
        plan.select(&name, &FlatSig::default());
    }
    let ac = JsName::new("ac".to_owned());
    let bc = JsName::new("bc".to_owned());
    plan.callers.insert(item(1), BTreeSet::from([ac.clone()]));
    plan.callers.insert(item(2), BTreeSet::from([bc.clone()]));
    plan.callers.insert(item(3), BTreeSet::from([ac, bc]));

    assert_eq!(
        plan.economic_groups(),
        vec![BTreeSet::from([item(1), item(2), item(3)])]
    );
}

fn flat_result_sig() -> FlatSig {
    FlatSig {
        params: vec![None],
        result: Some(flat_result(&interns().types, PAIR).expect("a pair result")),
        spill: 1,
        returns_out_field: false,
    }
}

fn link_item(name: &str, refs: &[&str]) -> LinkItem {
    LinkItem {
        name: JsName::new(name.to_owned()),
        kind: ItemKind::Fn,
        js: String::new(),
        refs: refs.iter().map(|r| JsName::new((*r).to_owned())).collect(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: String::new(),
        crate_name: "test".to_owned(),
        order: None,
        map: ItemMap::default(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    }
}

fn ineligible_of(op: OpKind) -> BTreeSet<ItemId> {
    let interns = interns();
    let mut scan = RefScan::default();
    refs::scan_op(
        &callee(),
        &Op {
            result: None,
            origin: NO_ORIGIN,
            kind: op,
        },
        Some(&interns.types),
        &mut scan,
    );
    scan.ineligible()
}

#[test]
fn a_reference_to_an_aggregate_is_a_flat_parameter_and_a_scalar_is_not() {
    let interns = interns();
    let flat = flat_param(&interns.types, REF_PAIR, true).expect("a reference to a tuple");
    assert_eq!(
        flat.kind,
        FlatParamKind::Aggregate {
            field_tys: vec![U32, U32],
            passed: vec![0, 1],
            access: ParamAccess::MutRef { written: true },
        }
    );
    assert!(flat.by_ref() && flat.mutable() && flat.written());
    assert_eq!(
        flat_param(&interns.types, PAIR, true).map(|flat| flat.by_ref()),
        Some(false)
    );
    assert_eq!(flat_param(&interns.types, U32, true), None);
}

#[test]
fn an_exported_callee_is_refused() {
    assert_eq!(
        refuses(
            Some(&Linkage::exported("step".to_owned())),
            None,
            &callee(),
            false,
            &BTreeSet::new()
        ),
        Some(BoundaryDecline::Exported)
    );
}

#[test]
fn a_spread_or_track_caller_abi_is_refused() {
    let mut spread = callee();
    spread.sig.abi.spread = Some(SpreadArgument {
        local: LocalId(1),
        arity: 2,
    });
    assert_eq!(
        refuses(
            Some(&Linkage::internal()),
            None,
            &spread,
            false,
            &BTreeSet::new()
        ),
        Some(BoundaryDecline::SpreadAbi)
    );
    let mut tracked = callee();
    tracked.sig.abi.caller_location = true;
    assert_eq!(
        refuses(
            Some(&Linkage::internal()),
            None,
            &tracked,
            false,
            &BTreeSet::new()
        ),
        Some(BoundaryDecline::SpreadAbi)
    );
}

#[test]
fn a_chunked_program_refuses_the_whole_plan() {
    assert_eq!(
        refuses(
            Some(&Linkage::internal()),
            None,
            &callee(),
            true,
            &BTreeSet::new()
        ),
        Some(BoundaryDecline::Chunks)
    );
}

#[test]
fn drop_glue_is_refused_because_host_lowerings_name_it_by_type() {
    assert_eq!(
        refuses(
            Some(&Linkage::internal()),
            Some(BodySemantic::DropGlue(PAIR)),
            &callee(),
            false,
            &BTreeSet::new()
        ),
        Some(BoundaryDecline::NonCallReference)
    );
}

#[test]
fn an_item_named_outside_a_direct_call_is_refused() {
    let ineligible = BTreeSet::from([item(1)]);
    assert_eq!(
        refuses(
            Some(&Linkage::internal()),
            None,
            &callee(),
            false,
            &ineligible
        ),
        Some(BoundaryDecline::NonCallReference)
    );
    assert_eq!(
        refuses(
            Some(&Linkage::internal()),
            None,
            &callee(),
            false,
            &BTreeSet::new()
        ),
        None
    );
}

#[test]
fn a_vtable_entry_keeps_its_item_out_of_the_plan() {
    // A vtable has no `Func`, so it is never in `funcs`: its `refs` name the method.
    let table = BTreeMap::from([
        (JsName::new("vt".to_owned()), link_item("vt", &["step"])),
        (JsName::new("step".to_owned()), link_item("step", &[])),
    ]);
    let funcs = BTreeMap::from([(JsName::new("step".to_owned()), Ok(callee()))]);
    let by_name = BTreeMap::from([("step", item(1))]);
    assert_eq!(
        refs::named_by_unlowered(&table, &funcs, &by_name),
        BTreeSet::from([item(1)])
    );
}

#[test]
fn a_function_constant_names_its_item() {
    assert_eq!(
        ineligible_of(OpKind::Const(ConstTree::Aggregate(vec![
            ConstTree::Global {
                target: GlobalReference::Function {
                    item: item(9),
                    link_name: StrId(0),
                },
                offset: 0,
            }
        ]))),
        BTreeSet::from([item(9)])
    );
}

#[test]
fn a_reified_function_pointer_names_its_item() {
    assert_eq!(
        ineligible_of(OpKind::Cast {
            kind: CastKind::PointerCoercion,
            v: ValueId(0),
            from: FN_DEF,
            to: U32,
        }),
        BTreeSet::from([item(9)])
    );
}

#[test]
fn an_intrinsic_fallback_names_its_item() {
    assert_eq!(
        ineligible_of(OpKind::Intrinsic {
            name: StrId(0),
            types: Vec::new(),
            fallback: Some(DirectItemRef {
                item: item(9),
                link_name: StrId(0),
            }),
            args: Vec::new(),
            track_caller: None,
        }),
        BTreeSet::from([item(9)])
    );
}

#[test]
fn a_host_call_names_its_callee_and_its_helpers() {
    assert_eq!(
        ineligible_of(OpKind::Call {
            callee: Callee::Direct {
                item: item(1),
                link_name: StrId(0),
            },
            function: None,
            args: Vec::new(),
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: Some(HostPayload::Marker {
                section: StrId(0),
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: 0,
                },
                helpers: vec![(
                    StrId(0),
                    DirectItemRef {
                        item: item(9),
                        link_name: StrId(0),
                    },
                )],
            }),
        }),
        BTreeSet::from([item(1), item(9)])
    );
}

#[test]
fn a_host_call_names_the_body_of_a_closure_argument() {
    // `lower/js/host.rs::closure_item` resolves the thunk from the argument's own type.
    let interns = interns();
    let mut scan = RefScan::default();
    let mut builder = FuncBuilder::new(item(2), signature(vec![FN_DEF], UNIT));
    let closure = builder.param(FN_DEF, NO_ORIGIN);
    let func = builder.finish();
    refs::scan_op(
        &func,
        &Op {
            result: None,
            origin: NO_ORIGIN,
            kind: OpKind::Call {
                callee: Callee::Runtime {
                    member: StrId(0),
                    sysroot: true,
                },
                function: None,
                args: vec![closure],
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: Some(HostPayload::Map { marker: StrId(0) }),
            },
        },
        Some(&interns.types),
        &mut scan,
    );
    let ineligible = scan.ineligible();
    // The body named by the type's own definition and instantiation, and the item the `FnDef`
    // resolves to.
    assert!(ineligible.contains(&ItemId {
        definition: DefinitionId {
            crate_id: 0,
            local_hash: 9,
        },
        instantiation: FN_DEF.0,
    }));
    assert!(ineligible.contains(&item(9)));
}

#[test]
fn a_plain_direct_call_names_nothing() {
    assert!(ineligible_of(OpKind::Call {
        callee: Callee::Direct {
            item: item(1),
            link_name: StrId(0),
        },
        function: None,
        args: Vec::new(),
        track_caller: None,
        semantic: crate::jir::semantic::CallSemantic::Ordinary,
        host: None,
    })
    .is_empty());
}

#[test]
fn every_compiler_decline_maps_to_a_physical_terminal_reason() {
    for decline in Decline::catalog() {
        let reason = terminal_reason(decline.code());
        assert!(
            [
                "not-considered.",
                "legality.",
                "boundary.",
                "cost.",
                "lower.",
                "validation.",
            ]
            .iter()
            .any(|prefix| reason.starts_with(prefix)),
            "{} mapped to {reason}",
            decline.code()
        );
    }
    assert_eq!(
        terminal_reason("encode.constant-unreadable"),
        "lower.encode.constant-unreadable"
    );
    assert_eq!(
        terminal_reason("merge.summary-conflict"),
        "validation.merge.summary-conflict"
    );
}

#[test]
fn the_callee_rewrite_takes_fields_and_returns_out_fields() {
    let interns = interns();
    let mut func = callee();
    func.verify(&interns.types).expect("the fixture verifies");
    rewrite_callee(&mut func, &flat_sig());
    func.verify(&interns.types)
        .expect("the rewritten callee verifies");
    assert_eq!(func.params.len(), 2);
    assert_eq!(func.sig.abi.parameters, 2);
    assert_eq!(func.sig.result, U32);
    assert_eq!(func.sig.arguments, vec![U32, U32]);
    assert!(matches!(
        func.blocks[0].ops[0].kind,
        OpKind::Aggregate { ty: PAIR, .. }
    ));
    assert!(matches!(
        func.blocks[0].ops[1].kind,
        OpKind::BorrowView { .. }
    ));
    let spills = func.blocks[0]
        .ops
        .iter()
        .filter(|op| matches!(op.kind, OpKind::SpillWrite { index: 1, .. }))
        .count();
    assert_eq!(spills, 1);
    assert!(matches!(func.blocks[0].term, RTerm::Return(Some(_))));
}

#[test]
fn the_caller_rewrite_passes_fields_and_reads_the_spill_back() {
    let interns = interns();
    let mut plan = AbiPlan {
        registers: 1,
        ..AbiPlan::default()
    };
    plan.flat.insert(item(1), flat_sig());
    let mut func = caller();
    func.verify(&interns.types).expect("the fixture verifies");
    rewrite_calls(&mut func, &plan, Some(&interns.types));
    func.verify(&interns.types)
        .expect("the rewritten caller verifies");
    let call = func.blocks[0]
        .ops
        .iter()
        .find(|op| matches!(op.kind, OpKind::Call { .. }))
        .expect("the call is still there");
    let OpKind::Call { args, .. } = &call.kind else {
        unreachable!()
    };
    assert_eq!(args.len(), 2);
    assert!(call.result.is_some());
    assert_eq!(
        func.blocks[0]
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OpKind::SpillRead(1)))
            .count(),
        1
    );
    assert_eq!(
        func.blocks[0]
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OpKind::StoreThrough { .. }))
            .count(),
        2
    );
}

#[test]
fn the_callee_rewrite_returns_an_aggregate_as_lanes() {
    let interns = interns();
    let mut func = result_callee();
    func.verify(&interns.types).expect("the fixture verifies");
    rewrite_callee(&mut func, &flat_result_sig());
    func.verify(&interns.types)
        .expect("the rewritten result callee verifies");
    assert_eq!(func.sig.result, U32);
    assert!(matches!(func.blocks[0].term, RTerm::Return(Some(_))));
    assert_eq!(
        func.blocks[0]
            .ops
            .iter()
            .filter(|op| matches!(op.kind, OpKind::SpillWrite { index: 1, .. }))
            .count(),
        1
    );
}

#[test]
fn the_caller_rewrite_rebuilds_a_flat_result_for_sroa() {
    let interns = interns();
    let mut plan = AbiPlan {
        registers: 1,
        ..AbiPlan::default()
    };
    plan.flat.insert(item(3), flat_result_sig());
    let mut func = result_caller();
    let aggregate = func.blocks[0].ops[0].result.expect("the original result");
    rewrite_calls(&mut func, &plan, Some(&interns.types));
    func.verify(&interns.types)
        .expect("the rewritten result caller verifies");
    let call = &func.blocks[0].ops[0];
    assert!(call.result.is_some_and(|value| value != aggregate));
    assert!(func.blocks[0]
        .ops
        .iter()
        .any(|op| matches!(op.kind, OpKind::SpillRead(1))));
    assert!(func.blocks[0].ops.iter().any(|op| {
        op.result == Some(aggregate) && matches!(op.kind, OpKind::Aggregate { ty: PAIR, .. })
    }));
}

#[test]
fn only_field_consumers_admit_a_flat_result() {
    let result = flat_result(&interns().types, PAIR).expect("a pair result");
    let field_only = result_caller();
    let use_ =
        field_only_value(&field_only, ValueId(1), &result, &BTreeMap::new()).expect("field-only");
    assert!(use_.read && use_.forwards.is_empty());

    let mut returned_whole = result_caller();
    returned_whole.sig.result = PAIR;
    returned_whole.blocks[0].term = RTerm::Return(Some(ValueId(1)));
    assert!(field_only_value(&returned_whole, ValueId(1), &result, &BTreeMap::new()).is_none());
}

#[test]
fn a_result_may_forward_whole_only_into_a_selected_product_parameter() {
    let types = &interns().types;
    let result = flat_result(types, PAIR).expect("a pair result");
    let by_value = flat_param(types, PAIR, false).expect("a pair parameter");
    let potential = BTreeMap::from([(item(5), vec![Some(by_value)])]);
    let use_ = field_only_value(&forwarding_result_caller(), ValueId(1), &result, &potential)
        .expect("a compatible direct-call forward");
    assert!(!use_.read);
    assert_eq!(use_.forwards, BTreeSet::from([(item(5), 0)]));

    let selected = BTreeMap::from([(item(5), vec![true])]);
    assert!(result_forwards_survive(&use_.forwards, &selected));
    let declined = BTreeMap::from([(item(5), vec![false])]);
    assert!(!result_forwards_survive(&use_.forwards, &declined));
}

#[test]
fn a_spill_register_is_a_module_scope_let_the_walk_can_follow() {
    let plan = AbiPlan {
        registers: 2,
        spill_names: allocate_spill_names(2, &BTreeMap::new()).expect("two free names"),
        ..AbiPlan::default()
    };
    let items = plan.spill_items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name.as_str(), "$r1");
    assert_eq!(items[1].js, "\nlet $r2;\n");
    assert!(items[0].kind.is_binding());
}

/// The program may bind `$r1` itself — `#[unsafe(export_name = "$r1")]` is all it takes, and
/// a `link_dedup` winner keeps whichever name the program gave it. Overwriting the item with a
/// `let` would delete it, so the register steps aside instead.
#[test]
fn a_register_steps_past_a_name_the_program_already_uses() {
    let mut table = BTreeMap::new();
    table.insert(JsName::new("$r1".to_owned()), link_item("$r1", &[]));
    // Not defined by any item, only *mentioned* by one — an inline `js!` block naming a
    // global, say. A module-scope `let` of that name would capture the mention.
    table.insert(
        JsName::new("user".to_owned()),
        link_item("user", &["$r2", "$r1$1"]),
    );
    let names = allocate_spill_names(2, &table).expect("names are available");
    assert_eq!(
        names.iter().map(JsName::as_str).collect::<Vec<_>>(),
        ["$r1$2", "$r2$1"]
    );
    // A `$h`-shaped name is one `link::stub_missing` would synthesize a throwing body for, so
    // a register must never be spelled like one. None of the candidates can be.
    assert!(names.iter().all(|name| !name.as_str().contains("$h")));
}

/// The belt-and-braces path: nowhere left to put a register at all.
#[test]
fn a_register_with_no_name_left_refuses_the_plan() {
    let mut table = BTreeMap::new();
    for name in
        std::iter::once("$r1".to_owned()).chain((1..64).map(|suffix| format!("$r1${suffix}")))
    {
        table.insert(JsName::new(name.clone()), link_item(&name, &[]));
    }
    assert!(allocate_spill_names(1, &table).is_none());
    // Which is a decline of the whole plan, recorded per callee like any other.
    assert_eq!(
        Decline::Boundary(BoundaryDecline::GeneratedNameCollision).code(),
        "boundary.generated-name-collision"
    );
}

#[test]
fn the_sidecar_block_names_every_flat_and_declined_item() {
    let mut plan = AbiPlan {
        registers: 1,
        ..AbiPlan::default()
    };
    plan.flat.insert(item(1), flat_sig());
    plan.names.insert(item(1), JsName::new("step".to_owned()));
    plan.declined.insert(
        JsName::new("keep".to_owned()),
        Decline::Boundary(BoundaryDecline::ParamEscapes),
    );
    assert_eq!(
        plan.json(true).expect("a plan was made"),
        "{\"applied\":true,\"registers\":1,\"flat\":[{\"item\":\"step\",\"params\":[2],\"result\":0,\"spill\":1,\"canonical_wrapper\":false}],\
         \"declined\":[{\"item\":\"keep\",\"reason\":\"boundary.param-escapes\"}]}"
    );
    assert!(AbiPlan::default().json(true).is_none());
}

#[test]
fn a_small_enum_uses_the_common_tagged_product_shape() {
    let interns = interns();
    let param = flat_param(&interns.types, RESULT, true).expect("a tagged result parameter");
    assert_eq!(param.fields(), 2);
    assert_eq!(
        param.tagged().map(|shape| shape.fields.as_slice()),
        Some(&[(0, 0), (1, 0)][..])
    );
    let result = flat_result(&interns.types, RESULT).expect("a tagged result value");
    assert_eq!(result.fields(), 2);
    assert_eq!(result.tagged(), param.tagged());
    assert!(scalar_leaf_result(&interns.types, U32));
    assert!(!scalar_leaf_result(&interns.types, RESULT));
    // The zero-sized and the non-aggregate shapes have nothing to flatten either.
    assert_eq!(flat_param(&interns.types, UNIT, true), None);
    assert_eq!(flat_param(&interns.types, FN_DEF, true), None);
}

#[test]
fn a_zero_sized_field_is_rebuilt_rather_than_passed() {
    let interns = interns();
    let flat = flat_param(&interns.types, CTX, true).expect("a tuple with a unit field");
    assert_eq!(
        flat.kind,
        FlatParamKind::Aggregate {
            field_tys: vec![U32, UNIT],
            passed: vec![0],
            access: ParamAccess::ByValue,
        }
    );
    assert_eq!(flat.fields(), 1);
}

#[test]
fn an_unwritten_mutable_reference_needs_no_copy_back() {
    let interns = interns();
    let read_only = flat_param(&interns.types, REF_PAIR, false).expect("a reference");
    assert!(read_only.mutable() && !read_only.written());
    let sig = FlatSig {
        params: vec![Some(read_only)],
        result: None,
        spill: 0,
        returns_out_field: false,
    };
    assert!(sig.out_fields().is_empty());
    assert_eq!(flat_sig().out_fields(), vec![(0, 0), (0, 1)]);
}

#[test]
fn param_use_reports_whether_the_body_writes_through_the_reference() {
    use crate::analysis::param_use::{self, ParamKind};
    let interns = interns();
    let kinds = [Some(ParamKind::ByRef)];
    // `callee` writes `(*p).1`.
    let written = param_use::summarize(&callee(), &interns.types, &kinds);
    assert!(!written[0].escapes && written[0].written);
    // A body that only reads a field does not.
    let mut builder = FuncBuilder::new(item(3), signature(vec![REF_PAIR], U32));
    let p = builder.param(REF_PAIR, NO_ORIGIN);
    let d = builder.op(FuncBuilder::ENTRY, OpKind::Deref(p), PAIR, NO_ORIGIN);
    let high = builder.op(
        FuncBuilder::ENTRY,
        OpKind::Field {
            base: d,
            variant: None,
            field: 0,
        },
        U32,
        NO_ORIGIN,
    );
    builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(high)));
    let read = param_use::summarize(&builder.finish(), &interns.types, &kinds);
    assert!(!read[0].escapes && !read[0].written);
}

#[test]
fn a_tracked_caller_pins_its_callee_and_reports_its_own_reason() {
    // The reason is the plan's, and it is not the callee's own ABI that is at fault.
    assert_eq!(
        BoundaryDecline::TrackedCaller.code(),
        "boundary.tracked-caller"
    );
    let mut tracked = caller();
    tracked.sig.abi.caller_location = true;
    // The scan records the callee of every direct call a `#[track_caller]` body makes.
    let mut pinned = BTreeSet::new();
    for block in &tracked.blocks {
        for op in &block.ops {
            if let OpKind::Call {
                callee: Callee::Direct { item, .. },
                host: None,
                ..
            } = &op.kind
            {
                if tracked.sig.abi.caller_location {
                    pinned.insert(*item);
                }
            }
        }
    }
    assert_eq!(pinned, BTreeSet::from([item(1)]));
}

#[test]
fn a_caller_that_cannot_spell_the_fields_does_not_agree() {
    let interns = interns();
    let shapes = [Some(flat_pair())];
    assert!(shapes_agree(&interns.types, &shapes));
    // A table that does not know the aggregate at all cannot spell its fields.
    let mut other = crate::jir::types::TypeTable::new();
    other.insert(descriptor(
        U32,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
    ));
    assert!(!shapes_agree(&other, &shapes));
    // And one with no `&mut u32` still agrees: the field's own type stands in for the view.
    let mut no_reference = crate::jir::types::TypeTable::new();
    no_reference.insert(descriptor(
        U32,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
    ));
    no_reference.insert(descriptor(PAIR, TypeKind::Tuple(vec![U32, U32])));
    assert!(shapes_agree(&no_reference, &shapes));
    assert_eq!(store_view_ty(&interns.types, U32), Some(REF_U32));
    assert_eq!(store_view_ty(&no_reference, U32), Some(U32));
    assert_eq!(store_view_ty(&interns.types, REF_PAIR), None);
}

#[test]
fn a_parameter_forwarded_to_one_that_is_not_flat_is_not_flat_either() {
    let escaping = ParamUse {
        escapes: true,
        ..ParamUse::default()
    };
    let forwards = |to: ItemId| ParamUse {
        forwards: BTreeSet::from([(to, 0)]),
        ..ParamUse::default()
    };
    let candidate = |name: &str, use_: ParamUse| Candidate {
        name: JsName::new(name.to_owned()),
        returns_zst: true,
        shapes: vec![Some(flat_pair())],
        result: None,
        result_forwards: BTreeSet::new(),
        result_forward_sites: 0,
        result_hot_forward: false,
        tagged_leaf_consumer: false,
        uses: vec![use_],
        canonical: false,
    };
    // 3 -> 2 -> 1, and 1's parameter escapes: none of the three is flat.
    let candidates = BTreeMap::from([
        (item(1), candidate("one", escaping)),
        (item(2), candidate("two", forwards(item(1)))),
        (item(3), candidate("three", forwards(item(2)))),
    ]);
    let flat = close_over_forwards(&candidates);
    assert_eq!(flat[&item(1)], vec![false]);
    assert_eq!(flat[&item(2)], vec![false]);
    assert_eq!(flat[&item(3)], vec![false]);
    // A forward to a parameter that stays flat keeps its caller flat.
    let candidates = BTreeMap::from([
        (item(1), candidate("one", ParamUse::default())),
        (item(2), candidate("two", forwards(item(1)))),
    ]);
    let flat = close_over_forwards(&candidates);
    assert_eq!(flat[&item(1)], vec![true]);
    assert_eq!(flat[&item(2)], vec![true]);
    // A forward to an item that is not a candidate at all is not flat.
    let candidates = BTreeMap::from([(item(2), candidate("two", forwards(item(9))))]);
    assert_eq!(close_over_forwards(&candidates)[&item(2)], vec![false]);
}

#[test]
fn tagged_admission_requires_a_result_forward_into_a_leaf_scalar_consumer() {
    let interns = interns();
    let param = flat_param(&interns.types, RESULT, false).expect("a tagged parameter");
    let result = flat_result(&interns.types, RESULT).expect("a tagged result");
    let candidate = |name: &str,
                     result: Option<FlatResult>,
                     forwards: BTreeSet<(ItemId, usize)>,
                     use_: ParamUse| {
        let forward_sites = if forwards.is_empty() { 0 } else { 2 };
        Candidate {
            name: JsName::new(name.to_owned()),
            returns_zst: false,
            shapes: vec![Some(param.clone())],
            result,
            result_forwards: forwards,
            result_forward_sites: forward_sites,
            result_hot_forward: false,
            tagged_leaf_consumer: name == "consumer",
            uses: vec![use_],
            canonical: false,
        }
    };
    let mut candidates = BTreeMap::from([
        (
            item(1),
            candidate(
                "producer",
                Some(result.clone()),
                BTreeSet::from([(item(2), 0)]),
                ParamUse::default(),
            ),
        ),
        (
            item(2),
            candidate(
                "middle",
                None,
                BTreeSet::new(),
                ParamUse {
                    forwards: BTreeSet::from([(item(3), 0)]),
                    ..ParamUse::default()
                },
            ),
        ),
        (
            item(3),
            candidate("consumer", None, BTreeSet::new(), ParamUse::default()),
        ),
        (
            item(4),
            candidate(
                "isolated",
                Some(result),
                BTreeSet::new(),
                ParamUse::default(),
            ),
        ),
    ]);
    let mut cold_single = candidate(
        "cold_single",
        Some(flat_result(&interns.types, RESULT).expect("a tagged result")),
        BTreeSet::from([(item(3), 0)]),
        ParamUse::default(),
    );
    cold_single.result_forward_sites = 1;
    candidates.insert(item(5), cold_single);

    let chains = tagged_chain_members(&candidates);
    assert_eq!(chains.results, BTreeSet::from([item(1)]));
    assert_eq!(chains.params, BTreeSet::from([(item(2), 0), (item(3), 0)]));
    assert!(!chains.results.contains(&item(4)));
    assert!(!chains.results.contains(&item(5)));
    assert!(!chains.params.contains(&(item(4), 0)));
}

#[test]
fn a_retry_keeps_what_the_plan_refused() {
    let mut plan = AbiPlan {
        registers: 1,
        ..AbiPlan::default()
    };
    plan.flat.insert(item(1), flat_sig());
    plan.names.insert(item(1), JsName::new("step".to_owned()));
    plan.touched.insert(JsName::new("step".to_owned()));
    plan.declined.insert(
        JsName::new("keep".to_owned()),
        Decline::Boundary(BoundaryDecline::ParamEscapes),
    );
    let emptied = plan.emptied();
    assert!(emptied.is_empty());
    assert!(!emptied.touches(&JsName::new("step".to_owned())));
    assert!(emptied.spill_items().is_empty());
    assert_eq!(
        emptied.json(true).expect("the refusals survive"),
        "{\"applied\":true,\"registers\":0,\"flat\":[],\"declined\":[{\"item\":\"keep\",\
         \"reason\":\"boundary.param-escapes\"}]}"
    );
}

#[test]
fn the_caller_stores_back_through_the_pointer_and_not_through_the_deref() {
    // Under JIR-R's value semantics a `Deref` yields a copy, so a `SetField` on it would not
    // reach the caller's storage; the interpreter is what says so.
    let interns = interns();
    let mut plan = AbiPlan {
        registers: 1,
        ..AbiPlan::default()
    };
    plan.flat.insert(item(1), flat_sig());
    let mut func = caller();
    rewrite_calls(&mut func, &plan, Some(&interns.types));
    func.verify(&interns.types).expect("verifies");
    assert!(!func.blocks[0]
        .ops
        .iter()
        .any(|op| matches!(op.kind, OpKind::SetField { .. })));
    let stores = func.blocks[0]
        .ops
        .iter()
        .filter(|op| matches!(op.kind, OpKind::StoreThrough { .. }))
        .count();
    assert_eq!(stores, 2);
    assert!(func.blocks[0].ops.iter().any(|op| matches!(
        &op.kind,
        OpKind::BorrowView { path, .. }
            if matches!(path.as_slice(), [Proj::Deref, Proj::Field { field: 1, .. }])
    )));
}

#[test]
fn scalar_ref_and_mut_enum_rewrite_verify_and_unbox() {
    let mut interns = interns();
    let i32_ty = TypeId(100);
    let ref_i32_ty = TypeId(101);
    let state_ty = TypeId(102);
    let ref_state_ty = TypeId(103);
    let mut i32_desc = descriptor(
        i32_ty,
        TypeKind::Integer {
            signed: true,
            bits: 32,
        },
    );
    i32_desc.js.int = Some(crate::jir::types::IntFacts {
        signed: true,
        bits: 32,
        repr: crate::jir::types::IntRepr::Number,
    });
    interns.types.insert(i32_desc);
    let mut ref_i32_desc = descriptor(
        ref_i32_ty,
        TypeKind::Reference {
            pointee: i32_ty,
            mutable: true,
        },
    );
    ref_i32_desc.js.pointer_form = Some(PointerForm::Slot);
    interns.types.insert(ref_i32_desc);
    let loading_name = StrId(interns.strings.len() as u32);
    interns.strings.push("Loading".to_owned());
    let ready_name = StrId(interns.strings.len() as u32);
    interns.strings.push("Ready".to_owned());
    let idle_name = StrId(interns.strings.len() as u32);
    interns.strings.push("Idle".to_owned());
    let p_name = StrId(interns.strings.len() as u32);
    interns.strings.push("progress".to_owned());
    let r_name = StrId(interns.strings.len() as u32);
    interns.strings.push("retries".to_owned());
    let v_name = StrId(interns.strings.len() as u32);
    interns.strings.push("value".to_owned());
    let mut state_desc = descriptor(
        state_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 0,
                local_hash: 42,
            },
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                crate::jir::types::VariantDescriptor {
                    name: idle_name,
                    fields: Vec::new(),
                    logical_discr: 0,
                    physical_tag: Some(crate::jir::types::NominalDiscriminant {
                        bits: 0,
                        bytes: 1,
                    }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
                crate::jir::types::VariantDescriptor {
                    name: loading_name,
                    fields: vec![
                        FieldDescriptor {
                            key: FieldKey::Name(p_name),
                            ty: i32_ty,
                            offset: 0,
                        },
                        FieldDescriptor {
                            key: FieldKey::Name(r_name),
                            ty: i32_ty,
                            offset: 4,
                        },
                    ],
                    logical_discr: 1,
                    physical_tag: Some(crate::jir::types::NominalDiscriminant {
                        bits: 1,
                        bytes: 1,
                    }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
                crate::jir::types::VariantDescriptor {
                    name: ready_name,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(v_name),
                        ty: i32_ty,
                        offset: 0,
                    }],
                    logical_discr: 2,
                    physical_tag: Some(crate::jir::types::NominalDiscriminant {
                        bits: 2,
                        bytes: 1,
                    }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
            ],
            niche: None,
        },
    );
    state_desc.js.is_indirect = true;
    state_desc.js.needs_clone = true;
    state_desc.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.types.insert(state_desc);
    interns.types.insert(descriptor(
        ref_state_ty,
        TypeKind::Reference {
            pointee: state_ty,
            mutable: true,
        },
    ));

    let mut callee_b = FuncBuilder::new(
        item(10),
        signature(vec![ref_state_ty, i32_ty, ref_i32_ty], UNIT),
    );
    let p_state = callee_b.param(ref_state_ty, NO_ORIGIN);
    let p_i = callee_b.param(i32_ty, NO_ORIGIN);
    let p_acc = callee_b.param(ref_i32_ty, NO_ORIGIN);
    let cur_s = callee_b.op(
        FuncBuilder::ENTRY,
        OpKind::Deref(p_state),
        state_ty,
        NO_ORIGIN,
    );
    let d = callee_b.op(
        FuncBuilder::ENTRY,
        OpKind::Discriminant(cur_s),
        i32_ty,
        NO_ORIGIN,
    );
    let arm_idle = callee_b.block();
    let arm_loading = callee_b.block();
    let arm_ready = callee_b.block();
    let ret_block = callee_b.block();
    callee_b.terminate(
        FuncBuilder::ENTRY,
        RTerm::Switch {
            scrut: d,
            scrut_ty: i32_ty,
            cases: vec![(0, arm_idle, Vec::new()), (1, arm_loading, Vec::new())],
            default: (arm_ready, Vec::new()),
        },
    );
    // Idle -> Ready { value: p_i }
    let agg0 = callee_b.op(
        arm_idle,
        OpKind::Aggregate {
            ty: state_ty,
            variant: 2,
            active_field: None,
            fields: vec![p_i],
        },
        state_ty,
        NO_ORIGIN,
    );
    callee_b.effect(
        arm_idle,
        OpKind::StoreThrough {
            ptr: p_state,
            v: agg0,
        },
        NO_ORIGIN,
    );
    callee_b.terminate(arm_idle, RTerm::Goto(ret_block, Vec::new()));

    // Loading -> Idle
    let agg1 = callee_b.op(
        arm_loading,
        OpKind::Aggregate {
            ty: state_ty,
            variant: 0,
            active_field: None,
            fields: Vec::new(),
        },
        state_ty,
        NO_ORIGIN,
    );
    callee_b.effect(
        arm_loading,
        OpKind::StoreThrough {
            ptr: p_state,
            v: agg1,
        },
        NO_ORIGIN,
    );
    callee_b.terminate(arm_loading, RTerm::Goto(ret_block, Vec::new()));

    // Ready -> *acc += value; *state = Idle
    let s_r = callee_b.op(arm_ready, OpKind::Deref(p_state), state_ty, NO_ORIGIN);
    let f_val = callee_b.op(
        arm_ready,
        OpKind::Field {
            base: s_r,
            variant: Some(2),
            field: 0,
        },
        i32_ty,
        NO_ORIGIN,
    );
    let cur_acc = callee_b.op(arm_ready, OpKind::Deref(p_acc), i32_ty, NO_ORIGIN);
    let new_acc = callee_b.op(
        arm_ready,
        OpKind::Binary {
            op: crate::jir::semantic::BinaryOp::Add,
            l: cur_acc,
            r: f_val,
            ty: i32_ty,
        },
        i32_ty,
        NO_ORIGIN,
    );
    let re_acc = callee_b.op(
        arm_ready,
        OpKind::BorrowView {
            root: Root::Value(p_acc),
            path: vec![Proj::Deref],
            mutable: true,
        },
        ref_i32_ty,
        NO_ORIGIN,
    );
    callee_b.effect(
        arm_ready,
        OpKind::StoreThrough {
            ptr: re_acc,
            v: new_acc,
        },
        NO_ORIGIN,
    );
    let agg2 = callee_b.op(
        arm_ready,
        OpKind::Aggregate {
            ty: state_ty,
            variant: 0,
            active_field: None,
            fields: Vec::new(),
        },
        state_ty,
        NO_ORIGIN,
    );
    callee_b.effect(
        arm_ready,
        OpKind::StoreThrough {
            ptr: p_state,
            v: agg2,
        },
        NO_ORIGIN,
    );
    callee_b.terminate(arm_ready, RTerm::Goto(ret_block, Vec::new()));

    callee_b.terminate(ret_block, RTerm::Return(None));
    let mut callee_func = callee_b.finish();

    let mut caller_b = FuncBuilder::new(item(11), signature(vec![i32_ty], i32_ty));
    let x = caller_b.param(i32_ty, NO_ORIGIN);
    let init_s = caller_b.op(
        FuncBuilder::ENTRY,
        OpKind::Aggregate {
            ty: state_ty,
            variant: 0,
            active_field: None,
            fields: Vec::new(),
        },
        state_ty,
        NO_ORIGIN,
    );
    let acc_cell = caller_b.cell(i32_ty, true, NO_ORIGIN);
    caller_b.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(acc_cell), NO_ORIGIN);
    caller_b.effect(
        FuncBuilder::ENTRY,
        OpKind::CellStore {
            cell: acc_cell,
            v: x,
        },
        NO_ORIGIN,
    );
    let s_ptr = caller_b.op(
        FuncBuilder::ENTRY,
        OpKind::BorrowView {
            root: Root::Value(init_s),
            path: Vec::new(),
            mutable: true,
        },
        ref_state_ty,
        NO_ORIGIN,
    );
    let a_ptr = caller_b.op(
        FuncBuilder::ENTRY,
        OpKind::BorrowView {
            root: Root::Cell(acc_cell),
            path: Vec::new(),
            mutable: true,
        },
        ref_i32_ty,
        NO_ORIGIN,
    );
    caller_b.effect(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(10),
                link_name: StrId(0),
            },
            function: None,
            args: vec![s_ptr, x, a_ptr],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        NO_ORIGIN,
    );
    let out_a = caller_b.op(
        FuncBuilder::ENTRY,
        OpKind::CellLoad(acc_cell),
        i32_ty,
        NO_ORIGIN,
    );
    caller_b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(out_a)));
    let mut caller_func = caller_b.finish();

    let p0 = flat_param_with(Some(&callee_func), &interns.types, ref_state_ty, true)
        .expect("mut_enum param");
    let p2 = flat_param_with(Some(&callee_func), &interns.types, ref_i32_ty, true)
        .expect("scalar_ref param");
    assert!(p0.is_mut_enum());
    assert!(p2.is_scalar_ref());
    let sig = FlatSig {
        params: vec![Some(p0), None, Some(p2)],
        result: None,
        spill: 4,
        returns_out_field: true,
    };
    let mut plan = AbiPlan {
        registers: 4,
        ..AbiPlan::default()
    };
    plan.flat.insert(item(10), sig.clone());
    plan.names.insert(item(10), JsName::new("step".to_owned()));

    rewrite_callee_with(&mut callee_func, &sig, Some(&interns.types));
    unbox_scalar_ref_cells(&mut callee_func, &interns.types);
    callee_func.verify(&interns.types).expect("callee verifies");
    assert!(!callee_func.cells[0].boxed, "callee cell unboxed");

    rewrite_calls(&mut caller_func, &plan, Some(&interns.types));
    unbox_scalar_ref_cells(&mut caller_func, &interns.types);
    caller_func.verify(&interns.types).expect("caller verifies");
    assert!(!caller_func.cells[0].boxed, "caller cell unboxed");

    let mut opts = Opts::default();
    opts.optimizer = crate::opts::OptimizerMode::On;
    let mut stats = crate::opt::OptStats::default();
    crate::opt::optimize(
        &mut callee_func,
        &interns.types,
        &interns,
        &opts,
        &mut stats,
    )
    .expect("callee optimizes");
    assert!(
        callee_func
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| matches!(op.kind, OpKind::Binary { .. })),
        "optimized callee kept Add in Ready branch: {callee_func:#?}"
    );
    crate::opt::optimize(
        &mut caller_func,
        &interns.types,
        &interns,
        &opts,
        &mut stats,
    )
    .expect("caller optimizes");

    let plan_callee = crate::lower::fuse::plan(&callee_func, None, &interns.types, &interns, &opts);
    crate::lower::structure::structure(&callee_func, &plan_callee, &interns.types, false)
        .expect("callee structures");
    let plan_caller = crate::lower::fuse::plan(&caller_func, None, &interns.types, &interns, &opts);
    crate::lower::structure::structure(&caller_func, &plan_caller, &interns.types, false)
        .expect("caller structures");
}

#[test]
fn pin_transparent_ref_is_not_scalar_or_mut_enum_flattened() {
    let mut types = TypeTable::default();
    let i32_ty = TypeId(200);
    let ref_i32_ty = TypeId(201);
    let pin_i32_ty = TypeId(202);
    let non_slot_ref_ty = TypeId(203);
    let mut i32_desc = descriptor(
        i32_ty,
        TypeKind::Integer {
            signed: true,
            bits: 32,
        },
    );
    i32_desc.js.int = Some(crate::jir::types::IntFacts {
        signed: true,
        bits: 32,
        repr: crate::jir::types::IntRepr::Number,
    });
    types.insert(i32_desc);

    let mut ref_i32_desc = descriptor(
        ref_i32_ty,
        TypeKind::Reference {
            pointee: i32_ty,
            mutable: true,
        },
    );
    ref_i32_desc.js.pointer_form = Some(PointerForm::Slot);
    types.insert(ref_i32_desc);

    // `Pin<&mut i32>` peels to `&mut i32` but is itself an ADT with `transparent = Some(...)`.
    let mut pin_i32_desc = descriptor(
        pin_i32_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 0,
                local_hash: 202,
            },
            args: vec![ref_i32_ty],
            form: NominalForm::Struct,
            variants: Vec::new(),
            niche: None,
        },
    );
    pin_i32_desc.js.transparent = Some(crate::jir::types::TransparentField {
        field: 0,
        ty: ref_i32_ty,
    });
    pin_i32_desc.js.peeled = ref_i32_ty;
    pin_i32_desc.js.pointer_form = Some(PointerForm::Thin);
    types.insert(pin_i32_desc);

    // A reference without `PointerForm::Slot` (e.g. Thin) is not scalar-ref flattened.
    let mut non_slot_desc = descriptor(
        non_slot_ref_ty,
        TypeKind::Reference {
            pointee: i32_ty,
            mutable: true,
        },
    );
    non_slot_desc.js.pointer_form = Some(PointerForm::Thin);
    types.insert(non_slot_desc);

    assert!(
        flat_param_with(None, &types, ref_i32_ty, true).is_some_and(|p| p.is_scalar_ref()),
        "plain &mut i32 with PointerForm::Slot must flatten"
    );
    assert!(
        flat_param_with(None, &types, pin_i32_ty, true).is_none_or(|p| !p.by_ref()),
        "Pin<&mut i32> must not flatten via peeled Reference"
    );
    assert!(
        flat_param_with(None, &types, non_slot_ref_ty, true).is_none_or(|p| !p.is_scalar_ref()),
        "Reference without PointerForm::Slot must not scalar-ref flatten"
    );
}

#[test]
fn mut_enum_rewrite_preserves_scalar_return_value() {
    let mut interns = interns();
    let i32_ty = TypeId(300);
    let state_ty = TypeId(301);
    let ref_state_ty = TypeId(302);
    let mut i32_desc = descriptor(
        i32_ty,
        TypeKind::Integer {
            signed: true,
            bits: 32,
        },
    );
    i32_desc.js.int = Some(crate::jir::types::IntFacts {
        signed: true,
        bits: 32,
        repr: crate::jir::types::IntRepr::Number,
    });
    interns.types.insert(i32_desc);
    let idle_name = StrId(interns.strings.len() as u32);
    interns.strings.push("Idle".to_owned());
    let active_name = StrId(interns.strings.len() as u32);
    interns.strings.push("Active".to_owned());
    let val_name = StrId(interns.strings.len() as u32);
    interns.strings.push("v".to_owned());
    let mut state_desc = descriptor(
        state_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 0,
                local_hash: 301,
            },
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                crate::jir::types::VariantDescriptor {
                    name: idle_name,
                    fields: Vec::new(),
                    logical_discr: 0,
                    physical_tag: Some(crate::jir::types::NominalDiscriminant {
                        bits: 0,
                        bytes: 1,
                    }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
                crate::jir::types::VariantDescriptor {
                    name: active_name,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(val_name),
                        ty: i32_ty,
                        offset: 0,
                    }],
                    logical_discr: 1,
                    physical_tag: Some(crate::jir::types::NominalDiscriminant {
                        bits: 1,
                        bytes: 1,
                    }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
            ],
            niche: None,
        },
    );
    state_desc.js.is_indirect = true;
    state_desc.js.needs_clone = true;
    state_desc.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.types.insert(state_desc);
    interns.types.insert(descriptor(
        ref_state_ty,
        TypeKind::Reference {
            pointee: state_ty,
            mutable: true,
        },
    ));

    let mut b = FuncBuilder::new(item(310), signature(vec![ref_state_ty, i32_ty], i32_ty));
    let p_s = b.param(ref_state_ty, NO_ORIGIN);
    let p_i = b.param(i32_ty, NO_ORIGIN);
    let agg = b.op(
        FuncBuilder::ENTRY,
        OpKind::Aggregate {
            ty: state_ty,
            variant: 1,
            active_field: None,
            fields: vec![p_i],
        },
        state_ty,
        NO_ORIGIN,
    );
    b.effect(
        FuncBuilder::ENTRY,
        OpKind::StoreThrough { ptr: p_s, v: agg },
        NO_ORIGIN,
    );
    b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(p_i)));
    let mut func = b.finish();

    let p0 =
        flat_param_with(Some(&func), &interns.types, ref_state_ty, true).expect("mut_enum param");
    let sig = FlatSig {
        params: vec![Some(p0), None],
        result: None,
        spill: 2,
        returns_out_field: false,
    };
    rewrite_callee_with(&mut func, &sig, Some(&interns.types));
    func.verify(&interns.types)
        .expect("callee returning scalar verifies");
    assert!(
        func.blocks
            .iter()
            .any(|block| matches!(block.term, RTerm::Return(Some(v)) if v == p_i)),
        "Return block after mut_enum split must keep Return(Some(p_i)): {:#?}",
        func.blocks
    );
}

#[test]
fn mutated_recursive_by_value_param_is_not_flattened() {
    let mut interns = interns();
    let u32_ty = TypeId(400);
    let inner_ty = TypeId(401);
    let outer_ty = TypeId(402);
    let mut_ref_outer_ty = TypeId(403);

    let mut u32_desc = descriptor(
        u32_ty,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
    );
    u32_desc.js.int = Some(crate::jir::types::IntFacts {
        signed: false,
        bits: 32,
        repr: crate::jir::types::IntRepr::Number,
    });
    interns.types.insert(u32_desc);

    let mut push_str = |s: &str| {
        let id = StrId(interns.strings.len() as u32);
        interns.strings.push(s.to_owned());
        id
    };
    let a_name = push_str("a");
    let b_name = push_str("b");
    let inner_name = push_str("inner");
    let extra_name = push_str("extra");
    let inner_v = push_str("Inner");
    let outer_v = push_str("Outer");
    let mutate_direct_link = push_str("mutate_outer_direct");
    let mutate_borrow_link = push_str("mutate_outer_borrow");
    let read_only_link = push_str("read_outer_only");
    let touch_ref_link = push_str("touch_outer_ref");

    let inner_fields = vec![
        FieldDescriptor {
            key: FieldKey::Name(a_name),
            ty: u32_ty,
            offset: 0,
        },
        FieldDescriptor {
            key: FieldKey::Name(b_name),
            ty: u32_ty,
            offset: 4,
        },
    ];
    let mut inner_desc = descriptor(
        inner_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 401,
            },
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![crate::jir::types::VariantDescriptor {
                name: inner_v,
                fields: inner_fields.clone(),
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
    );
    inner_desc.layout.size = Some(8);
    inner_desc.js.is_indirect = true;
    inner_desc.js.needs_clone = true;
    inner_desc.js.fields = inner_fields;
    interns.types.insert(inner_desc);

    let outer_fields = vec![
        FieldDescriptor {
            key: FieldKey::Name(inner_name),
            ty: inner_ty,
            offset: 0,
        },
        FieldDescriptor {
            key: FieldKey::Name(extra_name),
            ty: u32_ty,
            offset: 8,
        },
    ];
    let mut outer_desc = descriptor(
        outer_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 402,
            },
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![crate::jir::types::VariantDescriptor {
                name: outer_v,
                fields: outer_fields.clone(),
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
    );
    outer_desc.layout.size = Some(12);
    outer_desc.js.is_indirect = true;
    outer_desc.js.needs_clone = true;
    outer_desc.js.fields = outer_fields;
    interns.types.insert(outer_desc);

    interns.types.insert(descriptor(
        mut_ref_outer_ty,
        TypeKind::Reference {
            pointee: outer_ty,
            mutable: true,
        },
    ));

    // Confirm `Outer` has a `RecursiveShape`.
    let shape = recursive_shape(&interns.types, outer_ty).expect("nested struct is recursive");
    assert_eq!(shape.fields(), 3);

    // 1. Callee mutates `outer.extra = 0` directly via `SetField`.
    let mut cb = FuncBuilder::new(item(20), signature(vec![outer_ty], u32_ty));
    let p_outer = cb.param(outer_ty, NO_ORIGIN);
    let zero = cb.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        u32_ty,
        NO_ORIGIN,
    );
    cb.effect(
        FuncBuilder::ENTRY,
        OpKind::SetField {
            base: p_outer,
            variant: None,
            field: 1,
            v: zero,
        },
        NO_ORIGIN,
    );
    let read = cb.op(
        FuncBuilder::ENTRY,
        OpKind::Field {
            base: p_outer,
            variant: None,
            field: 1,
        },
        u32_ty,
        NO_ORIGIN,
    );
    cb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(read)));
    let mutate_direct_func = cb.finish();

    // 2. Callee borrows `&mut outer` and forwards it to a helper call.
    let mut bb = FuncBuilder::new(item(22), signature(vec![outer_ty], u32_ty));
    let p_outer_b = bb.param(outer_ty, NO_ORIGIN);
    let mut_view = bb.op(
        FuncBuilder::ENTRY,
        OpKind::BorrowView {
            root: Root::Value(p_outer_b),
            path: Vec::new(),
            mutable: true,
        },
        mut_ref_outer_ty,
        NO_ORIGIN,
    );
    bb.effect(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(24),
                link_name: touch_ref_link,
            },
            function: None,
            args: vec![mut_view],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        NO_ORIGIN,
    );
    let read_b = bb.op(
        FuncBuilder::ENTRY,
        OpKind::Field {
            base: p_outer_b,
            variant: None,
            field: 1,
        },
        u32_ty,
        NO_ORIGIN,
    );
    bb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(read_b)));
    let mutate_borrow_func = bb.finish();

    // 3. Control: read-only recursive by-value callee (should flatten!).
    let mut rb = FuncBuilder::new(item(23), signature(vec![outer_ty], u32_ty));
    let p_outer_r = rb.param(outer_ty, NO_ORIGIN);
    let read_r = rb.op(
        FuncBuilder::ENTRY,
        OpKind::Field {
            base: p_outer_r,
            variant: None,
            field: 1,
        },
        u32_ty,
        NO_ORIGIN,
    );
    rb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(read_r)));
    let read_only_func = rb.finish();

    // Helper taking `&mut Outer`.
    let mut tb = FuncBuilder::new(item(24), signature(vec![mut_ref_outer_ty], UNIT));
    let _p_ref = tb.param(mut_ref_outer_ty, NO_ORIGIN);
    tb.terminate(FuncBuilder::ENTRY, RTerm::Return(None));
    let touch_ref_func = tb.finish();

    // Caller constructs `Outer` and calls all three callees.
    let mut kb = FuncBuilder::new(item(21), signature(vec![u32_ty], u32_ty));
    let p_x = kb.param(u32_ty, NO_ORIGIN);
    let mut call_with_outer = |callee_item: ItemId, link_name: StrId| {
        let inner_v_id = kb.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: inner_ty,
                variant: 0,
                active_field: None,
                fields: vec![p_x, p_x],
            },
            inner_ty,
            NO_ORIGIN,
        );
        let outer_v_id = kb.op(
            FuncBuilder::ENTRY,
            OpKind::Aggregate {
                ty: outer_ty,
                variant: 0,
                active_field: None,
                fields: vec![inner_v_id, p_x],
            },
            outer_ty,
            NO_ORIGIN,
        );
        kb.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: callee_item,
                    link_name,
                },
                function: None,
                args: vec![outer_v_id],
                track_caller: None,
                semantic: crate::jir::semantic::CallSemantic::Ordinary,
                host: None,
            },
            u32_ty,
            NO_ORIGIN,
        )
    };
    let _r1 = call_with_outer(item(20), mutate_direct_link);
    let _r2 = call_with_outer(item(22), mutate_borrow_link);
    let r3 = call_with_outer(item(23), read_only_link);
    kb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r3)));
    let caller_func = kb.finish();

    let make_item = |name: &str, func: &Func, refs: &[&str]| {
        let mut it = link_item(name, refs);
        let body = crate::jir::semantic::Body {
            item: func.item,
            kind: crate::jir::semantic::BodyKind::Function,
            semantic: crate::jir::semantic::BodySemantic::Ordinary,
            signature: func.sig.clone(),
            locals: Vec::new(),
            blocks: Vec::new(),
        };
        let hash = crate::jir::summary::semantic_hash(&body, &interns);
        let physical = crate::jir::summary::PhysicalSummary::of(&body, &interns.types, hash);
        it.jir.summary = Some(crate::jir::summary::ItemSummary {
            key: crate::jir::summary::SemanticKey {
                link_name: name.to_owned(),
                item: Some(func.item),
            },
            body_hash: None,
            semantic_hash: Some(hash),
            availability: crate::jir::summary::Availability::Available,
            deps: refs.iter().map(|r| (*r).to_owned()).collect(),
            abi: None,
            physical,
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            fallback: rcgjs_object::object::BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::AGGREGATE,
        });
        it.jir.body = Some(body);
        it.jir.use_interns(interns.clone());
        it
    };
    let mut table = BTreeMap::new();
    table.insert(
        JsName::new("mutate_outer_direct".to_owned()),
        make_item("mutate_outer_direct", &mutate_direct_func, &[]),
    );
    table.insert(
        JsName::new("mutate_outer_borrow".to_owned()),
        make_item(
            "mutate_outer_borrow",
            &mutate_borrow_func,
            &["touch_outer_ref"],
        ),
    );
    table.insert(
        JsName::new("read_outer_only".to_owned()),
        make_item("read_outer_only", &read_only_func, &[]),
    );
    table.insert(
        JsName::new("touch_outer_ref".to_owned()),
        make_item("touch_outer_ref", &touch_ref_func, &[]),
    );
    table.insert(
        JsName::new("entry".to_owned()),
        make_item(
            "entry",
            &caller_func,
            &[
                "mutate_outer_direct",
                "mutate_outer_borrow",
                "read_outer_only",
            ],
        ),
    );
    let mut funcs = BTreeMap::new();
    funcs.insert(
        JsName::new("mutate_outer_direct".to_owned()),
        Ok(mutate_direct_func),
    );
    funcs.insert(
        JsName::new("mutate_outer_borrow".to_owned()),
        Ok(mutate_borrow_func),
    );
    funcs.insert(
        JsName::new("read_outer_only".to_owned()),
        Ok(read_only_func),
    );
    funcs.insert(
        JsName::new("touch_outer_ref".to_owned()),
        Ok(touch_ref_func),
    );
    funcs.insert(JsName::new("entry".to_owned()), Ok(caller_func));

    let opts = crate::opts::Opts {
        optimizer: crate::opts::OptimizerMode::On,
        flat_abi: true,
        ..crate::opts::Opts::default()
    };
    let plan = plan_excluding(&table, &funcs, &opts, &BTreeSet::new());
    assert!(
        plan.flat.contains_key(&item(23)),
        "control: read-only recursive by-value parameter should flatten"
    );
    assert!(
        !plan.flat.contains_key(&item(20)),
        "SetField-mutated recursive by-value parameter must not flatten"
    );
    assert!(
        !plan.flat.contains_key(&item(22)),
        "mutably-borrowed recursive by-value parameter must not flatten"
    );
}

#[test]
fn product_node_keeps_raw_slot_pointers_and_zst_refs_as_single_value() {
    let mut interns = interns();
    let u8_ty = TypeId(500);
    let raw_ptr_ty = TypeId(501);
    let non_null_ty = TypeId(502);
    let ref_u8_ty = TypeId(503);
    let ref_unit_ty = TypeId(504);
    let args_ty = TypeId(505);

    let mut u8_desc = descriptor(
        u8_ty,
        TypeKind::Integer {
            signed: false,
            bits: 8,
        },
    );
    u8_desc.layout.size = Some(1);
    interns.types.insert(u8_desc);

    let mut raw_desc = descriptor(
        raw_ptr_ty,
        TypeKind::RawPointer {
            pointee: u8_ty,
            mutable: false,
        },
    );
    raw_desc.layout.size = Some(4);
    raw_desc.js.pointer_form = Some(PointerForm::Slot);
    interns.types.insert(raw_desc);

    let mut push_str = |s: &str| {
        let id = StrId(interns.strings.len() as u32);
        interns.strings.push(s.to_owned());
        id
    };
    let ptr_name = push_str("pointer");
    let nn_v = push_str("NonNull");
    let tpl_name = push_str("template");
    let args_name = push_str("args");
    let args_v = push_str("Arguments");

    let nn_fields = vec![FieldDescriptor {
        key: FieldKey::Transparent,
        ty: raw_ptr_ty,
        offset: 0,
    }];
    let mut nn_desc = descriptor(
        non_null_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 502,
            },
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![crate::jir::types::VariantDescriptor {
                name: nn_v,
                fields: nn_fields.clone(),
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
    );
    nn_desc.layout.size = Some(4);
    nn_desc.js.transparent = Some(crate::jir::types::TransparentField {
        ty: raw_ptr_ty,
        field: 0,
    });
    nn_desc.js.pointer_form = Some(PointerForm::Slot);
    nn_desc.js.fields = nn_fields;
    interns.types.insert(nn_desc);

    let mut ref_u8_desc = descriptor(
        ref_u8_ty,
        TypeKind::Reference {
            pointee: u8_ty,
            mutable: false,
        },
    );
    ref_u8_desc.layout.size = Some(4);
    ref_u8_desc.js.pointer_form = Some(PointerForm::Slot);
    interns.types.insert(ref_u8_desc);

    let mut ref_unit_desc = descriptor(
        ref_unit_ty,
        TypeKind::Reference {
            pointee: UNIT,
            mutable: false,
        },
    );
    ref_unit_desc.layout.size = Some(4);
    ref_unit_desc.js.pointer_form = Some(PointerForm::Slot);
    interns.types.insert(ref_unit_desc);

    let args_fields = vec![
        FieldDescriptor {
            key: FieldKey::Name(tpl_name),
            ty: non_null_ty,
            offset: 0,
        },
        FieldDescriptor {
            key: FieldKey::Name(args_name),
            ty: raw_ptr_ty,
            offset: 4,
        },
    ];
    let mut args_desc = descriptor(
        args_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 505,
            },
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![crate::jir::types::VariantDescriptor {
                name: args_v,
                fields: args_fields.clone(),
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
    );
    args_desc.layout.size = Some(8);
    args_desc.js.is_indirect = true;
    args_desc.js.needs_clone = true;
    args_desc.js.fields = args_fields;
    interns.types.insert(args_desc);

    assert_eq!(
        product_node(&interns.types, raw_ptr_ty, 0),
        Some(ProductNode::Leaf {
            ty: raw_ptr_ty,
            components: vec![LaneComponent::Value],
        }),
        "raw PointerForm::Slot must stay a single Value lane (can hold bare int / tagged addr)"
    );
    assert_eq!(
        product_node(&interns.types, non_null_ty, 0),
        Some(ProductNode::Leaf {
            ty: non_null_ty,
            components: vec![LaneComponent::Value],
        }),
        "transparent NonNull<T> over RawPointer must stay a single Value lane"
    );
    assert_eq!(
        product_node(&interns.types, ref_unit_ty, 0),
        Some(ProductNode::Leaf {
            ty: ref_unit_ty,
            components: vec![LaneComponent::Value],
        }),
        "ZST reference &() with PointerForm::Slot must stay a single Value lane"
    );
    assert_eq!(
        product_node(&interns.types, ref_u8_ty, 0),
        Some(ProductNode::Leaf {
            ty: ref_u8_ty,
            components: vec![LaneComponent::Buffer, LaneComponent::Offset],
        }),
        "non-ZST &u8 reference with PointerForm::Slot still splits into (Buffer, Offset)"
    );
    assert!(
        recursive_shape(&interns.types, args_ty).is_none(),
        "struct of raw slot pointers (core::fmt::Arguments) must not recursively split into buf/off"
    );
    let _ = ptr_name;
}

#[test]
fn sentinel_result_shape_accepts_needs_drop_zst_err_and_rejects_boundaries() {
    let mut interns = interns();
    let s_ok = interns.intern_str("Ok");
    let s_err = interns.intern_str("Err");
    let s_f0 = interns.intern_str("_0");
    let t_drop_res = TypeId(900);
    let t_zst_err_res = TypeId(901);
    let t_host_res = TypeId(902);

    let make_res =
        |id: TypeId, ok_ty: TypeId, err_ty: TypeId, needs_drop: bool, boundary: PinnedBoundary| {
            let mut d = descriptor(
                id,
                TypeKind::Adt {
                    def: DefinitionId {
                        crate_id: 1,
                        local_hash: id.0 as u64,
                    },
                    args: vec![ok_ty, err_ty],
                    form: NominalForm::Enum,
                    variants: vec![
                        crate::jir::types::VariantDescriptor {
                            name: s_ok,
                            fields: vec![FieldDescriptor {
                                key: FieldKey::Name(s_f0),
                                ty: ok_ty,
                                offset: 4,
                            }],
                            logical_discr: 0,
                            physical_tag: Some(crate::jir::types::NominalDiscriminant {
                                bits: 0,
                                bytes: 1,
                            }),
                            result_ok: true,
                            saved_local_keys: Vec::new(),
                        },
                        crate::jir::types::VariantDescriptor {
                            name: s_err,
                            fields: vec![FieldDescriptor {
                                key: FieldKey::Name(s_f0),
                                ty: err_ty,
                                offset: 4,
                            }],
                            logical_discr: 1,
                            physical_tag: Some(crate::jir::types::NominalDiscriminant {
                                bits: 1,
                                bytes: 1,
                            }),
                            result_ok: false,
                            saved_local_keys: Vec::new(),
                        },
                    ],
                    niche: None,
                },
            );
            d.layout.size = Some(8);
            d.layout.needs_drop = needs_drop;
            d.meta.boundary = boundary;
            d.js.is_indirect = true;
            d.js.needs_clone = true;
            d.js.enum_repr = Some(EnumRepr::IntTag { fieldless: false });
            d
        };

    interns
        .types
        .insert(make_res(t_drop_res, U32, U32, true, PinnedBoundary::None));
    interns.types.insert(make_res(
        t_zst_err_res,
        U32,
        UNIT,
        true,
        PinnedBoundary::None,
    ));
    interns
        .types
        .insert(make_res(t_host_res, U32, U32, false, PinnedBoundary::Host));

    let drop_shape = sentinel_result_shape(&interns.types, t_drop_res)
        .expect("needs_drop Result must be eligible for SentinelResult");
    assert_eq!(drop_shape.ok_variant, 0);
    assert_eq!(drop_shape.ok_field, Some((0, U32)));
    assert_eq!(drop_shape.err_variant, 1);
    assert_eq!(drop_shape.err_field, Some((0, U32)));
    assert_eq!(drop_shape.fields(), 2);

    let zst_err_shape = sentinel_result_shape(&interns.types, t_zst_err_res)
        .expect("ZST-err Result must be eligible for SentinelResult");
    assert_eq!(zst_err_shape.err_field, None);
    assert_eq!(zst_err_shape.fields(), 1);

    assert!(
        sentinel_result_shape(&interns.types, t_host_res).is_none(),
        "Host-boundary enum must not use SentinelResult"
    );
}

#[test]
fn sentinel_result_plans_recursive_needs_drop_invoke_and_escaping_callers() {
    use crate::jir::semantic::{BasicBlock, BlockId, Body, BodyKind, Terminator};
    use crate::jir::summary::{Availability, ItemSummary, PhysicalSummary, SemanticKey};

    let mut interns = interns();
    let s_ok = interns.intern_str("Ok");
    let s_err = interns.intern_str("Err");
    let s_f0 = interns.intern_str("_0");
    let s_eval = interns.intern_str("eval");
    let t_drop_res = TypeId(910);

    let mut res_desc = descriptor(
        t_drop_res,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 910,
            },
            args: vec![U32, U32],
            form: NominalForm::Enum,
            variants: vec![
                crate::jir::types::VariantDescriptor {
                    name: s_ok,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(s_f0),
                        ty: U32,
                        offset: 4,
                    }],
                    logical_discr: 0,
                    physical_tag: Some(crate::jir::types::NominalDiscriminant {
                        bits: 0,
                        bytes: 1,
                    }),
                    result_ok: true,
                    saved_local_keys: Vec::new(),
                },
                crate::jir::types::VariantDescriptor {
                    name: s_err,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(s_f0),
                        ty: U32,
                        offset: 4,
                    }],
                    logical_discr: 1,
                    physical_tag: Some(crate::jir::types::NominalDiscriminant {
                        bits: 1,
                        bytes: 1,
                    }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
            ],
            niche: None,
        },
    );
    res_desc.layout.size = Some(8);
    res_desc.layout.needs_drop = true;
    res_desc.js.is_indirect = true;
    res_desc.js.needs_clone = true;
    res_desc.js.enum_repr = Some(EnumRepr::IntTag { fieldless: false });
    interns.types.insert(res_desc);
    let shared = std::sync::Arc::new(interns);

    // Recursive `eval(x: u32) -> Result<u32, u32>` with `RTerm::Invoke` self-call and `?`.
    let eval_id = item(10);
    let mut eb = FuncBuilder::new(eval_id, signature(vec![U32], t_drop_res));
    let x = eb.param(U32, NO_ORIGIN);
    let b_rec = eb.block();
    let b_norm = eb.block();
    let b_unwind = eb.block();
    let b_ok = eb.block();
    let b_err = eb.block();
    let b_base = eb.block();
    eb.terminate(
        FuncBuilder::ENTRY,
        RTerm::Switch {
            scrut: x,
            scrut_ty: U32,
            cases: vec![(0, b_base, Vec::new())],
            default: (b_rec, Vec::new()),
        },
    );
    let base_ok = eb.op(
        b_base,
        OpKind::Aggregate {
            ty: t_drop_res,
            variant: 0,
            active_field: None,
            fields: vec![x],
        },
        t_drop_res,
        NO_ORIGIN,
    );
    eb.terminate(b_base, RTerm::Return(Some(base_ok)));

    let sub = eb.op(
        b_rec,
        OpKind::Call {
            callee: Callee::Direct {
                item: eval_id,
                link_name: s_eval,
            },
            function: None,
            args: vec![x],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        t_drop_res,
        NO_ORIGIN,
    );
    eb.terminate(
        b_rec,
        RTerm::Invoke {
            normal: (b_norm, Vec::new()),
            unwind: (b_unwind, Vec::new()),
        },
    );
    eb.terminate(b_unwind, RTerm::Throw(None));
    let discr = eb.op(b_norm, OpKind::Discriminant(sub), U32, NO_ORIGIN);
    eb.terminate(
        b_norm,
        RTerm::Switch {
            scrut: discr,
            scrut_ty: U32,
            cases: vec![(0, b_ok, Vec::new())],
            default: (b_err, Vec::new()),
        },
    );
    let ok_v = eb.op(
        b_ok,
        OpKind::Field {
            base: sub,
            variant: Some(0),
            field: 0,
        },
        U32,
        NO_ORIGIN,
    );
    let ret_ok = eb.op(
        b_ok,
        OpKind::Aggregate {
            ty: t_drop_res,
            variant: 0,
            active_field: None,
            fields: vec![ok_v],
        },
        t_drop_res,
        NO_ORIGIN,
    );
    eb.terminate(b_ok, RTerm::Return(Some(ret_ok)));
    let err_v = eb.op(
        b_err,
        OpKind::Field {
            base: sub,
            variant: Some(1),
            field: 0,
        },
        U32,
        NO_ORIGIN,
    );
    let ret_err = eb.op(
        b_err,
        OpKind::Aggregate {
            ty: t_drop_res,
            variant: 1,
            active_field: None,
            fields: vec![err_v],
        },
        t_drop_res,
        NO_ORIGIN,
    );
    eb.terminate(b_err, RTerm::Return(Some(ret_err)));
    let eval_func = eb.finish();

    // Exported escaping caller `entry(x: u32) -> Result<u32, u32> { eval(x) }`.
    let entry_id = item(11);
    let mut mb = FuncBuilder::new(entry_id, signature(vec![U32], t_drop_res));
    let mx = mb.param(U32, NO_ORIGIN);
    let res = mb.op(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: eval_id,
                link_name: s_eval,
            },
            function: None,
            args: vec![mx],
            track_caller: None,
            semantic: crate::jir::semantic::CallSemantic::Ordinary,
            host: None,
        },
        t_drop_res,
        NO_ORIGIN,
    );
    mb.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(res)));
    let entry_func = mb.finish();

    let make_link_item = |name: &str, id: ItemId, sig: Signature, exported: bool, refs: &[&str]| {
        let mut it = link_item(name, refs);
        if exported {
            it.linkage = Linkage::exported(name.to_owned());
        }
        let body = Body {
            item: id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: sig,
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            }],
        };
        let hash = crate::jir::summary::semantic_hash(&body, &shared);
        let physical = PhysicalSummary::of(&body, &shared.types, hash);
        it.jir.body = Some(body);
        it.jir.summary = Some(ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: Some(id),
            },
            body_hash: None,
            semantic_hash: Some(hash),
            availability: Availability::Available,
            deps: refs.iter().map(|r| (*r).to_owned()).collect(),
            abi: None,
            physical,
            kind: ItemKind::Fn,
            linkage: it.linkage.clone(),
            order: None,
            dts: false,
            fallback: rcgjs_object::object::BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::AGGREGATE,
        });
        it.jir.use_shared_interns(shared.clone());
        it
    };

    let eval_name = JsName::new("eval".to_owned());
    let entry_name = JsName::new("entry".to_owned());
    let table = BTreeMap::from([
        (
            eval_name.clone(),
            make_link_item("eval", eval_id, eval_func.sig.clone(), false, &["eval"]),
        ),
        (
            entry_name.clone(),
            make_link_item("entry", entry_id, entry_func.sig.clone(), true, &["eval"]),
        ),
    ]);
    let funcs = BTreeMap::from([
        (eval_name.clone(), Ok(eval_func)),
        (entry_name, Ok(entry_func)),
    ]);

    let speed_opts = Opts {
        optimizer: crate::opts::OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseSpeed),
        flat_abi: true,
        ..Opts::default()
    };
    let plan = plan_excluding(&table, &funcs, &speed_opts, &BTreeSet::new());
    let sig = plan
        .call_signature(eval_id)
        .expect("recursive needs_drop Invoke Result callee must get SentinelResult");
    assert!(sig.result.as_ref().is_some_and(|r| r.sentinel().is_some()));
    assert_eq!(plan.registers, 1);
    assert_eq!(plan.sentinel_err_name().map(JsName::as_str), Some("$se"));
    let spills = plan.spill_items();
    assert_eq!(spills.len(), 2);
    assert!(spills.iter().any(|i| i.js.contains("let $r1;")));
    assert!(spills.iter().any(|i| i.js.contains("const $se = {};")));

    // Subset without sentinel items drops `$se`.
    let empty_sub = plan.subset(&BTreeSet::new());
    assert!(empty_sub.sentinel_err_name().is_none());
    assert!(empty_sub.spill_items().is_empty());

    // Non-ReleaseSpeed must not select SentinelResult.
    let bal_opts = Opts {
        optimizer: crate::opts::OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseBalanced),
        flat_abi: true,
        ..Opts::default()
    };
    let bal_plan = plan_excluding(&table, &funcs, &bal_opts, &BTreeSet::new());
    assert!(!bal_plan.has_sentinel_result());

    // Chunked mode must not select SentinelResult.
    let mut chunked_opts = speed_opts;
    chunked_opts
        .chunks
        .push(("extra".to_owned(), "entry".to_owned()));
    let chunked_plan = plan_excluding(&table, &funcs, &chunked_opts, &BTreeSet::new());
    assert!(!chunked_plan.has_sentinel_result());
}

#[test]
fn zero_offset_slice_specialization_positive_and_negative_cases() {
    use crate::jir::semantic::{
        BasicBlock, BinaryOp, BlockId, Body, BodyKind, CallSemantic, CastKind, GlobalReference,
        StandardOperation, Terminator, UnaryOp,
    };
    use crate::jir::summary::{Availability, ItemClass, ItemSummary, PhysicalSummary, SemanticKey};
    use crate::jir::types::{IntFacts, IntRepr};
    use std::sync::Arc;

    const SLICE_U32: TypeId = TypeId(10);
    const REF_SLICE_U32: TypeId = TypeId(11);
    const ARR4_U32: TypeId = TypeId(12);
    const REF_ARR4_U32: TypeId = TypeId(13);
    const PTR_U32: TypeId = TypeId(14);
    const VEC_U32: TypeId = TypeId(15);
    const REF_VEC_U32: TypeId = TypeId(16);
    const UNIT: TypeId = TypeId(17);

    let mut interns = Interns::default();
    let s_leaf = interns.intern_str("leaf");
    let s_mid = interns.intern_str("mid");
    let s_rec = interns.intern_str("rec");
    let s_mixed = interns.intern_str("mixed");
    let s_ptr_off = interns.intern_str("ptr_off");
    let s_exp_rec = interns.intern_str("exp_rec");
    let s_static_ok = interns.intern_str("static_ok");
    let s_static_bad = interns.intern_str("static_bad");
    let s_vec_as_slice = interns.intern_str("vec_as_slice");
    let s_static_arr = interns.intern_str("STATIC_ARR");
    let s_entry = interns.intern_str("entry");

    let js_facts = |peeled: TypeId,
                    int: Option<IntFacts>,
                    is_indirect: bool,
                    needs_clone: bool,
                    pointer_form: Option<PointerForm>| JsFacts {
        int,
        is_indirect,
        needs_clone,
        crosses_transparent_union: false,
        transparent: None,
        peeled,
        unsized_tail: None,
        enum_repr: None,
        is_primitive_enum: false,
        pointer_form,
        byte_element: false,
        fields: Vec::new(),
    };

    let scalar_u32 = TypeDescriptor {
        id: U32,
        name: None,
        kind: TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        layout: LayoutFacts {
            size: Some(4),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(
            U32,
            Some(IntFacts {
                signed: false,
                bits: 32,
                repr: IntRepr::Number,
            }),
            false,
            false,
            None,
        ),
    };
    let unit_desc = TypeDescriptor {
        id: UNIT,
        name: None,
        kind: TypeKind::Tuple(Vec::new()),
        layout: LayoutFacts {
            size: Some(0),
            align: Some(1),
            sized: true,
            zst: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(UNIT, None, false, false, None),
    };
    let slice_desc = TypeDescriptor {
        id: SLICE_U32,
        name: None,
        kind: TypeKind::Slice(U32),
        layout: LayoutFacts {
            sized: false,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(SLICE_U32, None, true, false, None),
    };
    let ref_slice_desc = TypeDescriptor {
        id: REF_SLICE_U32,
        name: None,
        kind: TypeKind::Reference {
            pointee: SLICE_U32,
            mutable: false,
        },
        layout: LayoutFacts {
            size: Some(8),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(
            REF_SLICE_U32,
            None,
            false,
            false,
            Some(PointerForm::FatSlice),
        ),
    };
    let arr_desc = TypeDescriptor {
        id: ARR4_U32,
        name: None,
        kind: TypeKind::Array {
            element: U32,
            length: 4,
        },
        layout: LayoutFacts {
            size: Some(16),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(ARR4_U32, None, true, true, None),
    };
    let ref_arr_desc = TypeDescriptor {
        id: REF_ARR4_U32,
        name: None,
        kind: TypeKind::Reference {
            pointee: ARR4_U32,
            mutable: false,
        },
        layout: LayoutFacts {
            size: Some(4),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(REF_ARR4_U32, None, false, false, Some(PointerForm::Thin)),
    };
    let ptr_u32_desc = TypeDescriptor {
        id: PTR_U32,
        name: None,
        kind: TypeKind::RawPointer {
            pointee: U32,
            mutable: false,
        },
        layout: LayoutFacts {
            size: Some(4),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(PTR_U32, None, false, false, Some(PointerForm::Slot)),
    };
    let vec_desc = TypeDescriptor {
        id: VEC_U32,
        name: None,
        kind: TypeKind::Opaque,
        layout: LayoutFacts {
            size: Some(12),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(VEC_U32, None, true, false, None),
    };
    let ref_vec_desc = TypeDescriptor {
        id: REF_VEC_U32,
        name: None,
        kind: TypeKind::Reference {
            pointee: VEC_U32,
            mutable: false,
        },
        layout: LayoutFacts {
            size: Some(4),
            align: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: js_facts(REF_VEC_U32, None, false, false, Some(PointerForm::Thin)),
    };
    for d in [
        scalar_u32,
        unit_desc,
        slice_desc,
        ref_slice_desc,
        arr_desc,
        ref_arr_desc,
        ptr_u32_desc,
        vec_desc,
        ref_vec_desc,
    ] {
        interns.types.insert(d);
    }
    let shared = Arc::new(interns);

    let build_len_fn = |id: ItemId| {
        let mut b = FuncBuilder::new(id, signature(vec![REF_SLICE_U32], U32));
        let s = b.param(REF_SLICE_U32, NO_ORIGIN);
        let len = b.op(
            FuncBuilder::ENTRY,
            OpKind::Unary {
                op: UnaryOp::PtrMetadata,
                v: s,
                ty: REF_SLICE_U32,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(len)));
        b.finish()
    };

    let leaf_id = item(100);
    let mid_id = item(101);
    let rec_id = item(102);
    let mixed_id = item(103);
    let ptr_off_id = item(104);
    let uncalled_id = item(105);
    let exp_rec_id = item(106);
    let static_ok_id = item(107);
    let static_bad_id = item(108);
    let vec_as_slice_id = item(109);
    let entry_id = item(110);
    let dead_inlined_id = item(111);

    let leaf_fn = build_len_fn(leaf_id);
    let mixed_fn = build_len_fn(mixed_id);
    let ptr_off_fn = build_len_fn(ptr_off_id);
    let uncalled_fn = build_len_fn(uncalled_id);
    let static_ok_fn = build_len_fn(static_ok_id);
    let static_bad_fn = build_len_fn(static_bad_id);

    // Unreachable dead helper (as left behind after JIR inlining): internal linkage, zero JIR
    // callers, still forwards its own slice param to `leaf`. Must NOT poison `leaf`!
    let dead_inlined_fn = {
        let mut b = FuncBuilder::new(dead_inlined_id, signature(vec![REF_SLICE_U32], U32));
        let s = b.param(REF_SLICE_U32, NO_ORIGIN);
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: leaf_id,
                    link_name: s_leaf,
                },
                function: None,
                args: vec![s],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        b.finish()
    };

    // `mid(s)` forwards `s` to `leaf(s)` and `rec(s)`.
    let mid_fn = {
        let mut b = FuncBuilder::new(mid_id, signature(vec![REF_SLICE_U32], U32));
        let s = b.param(REF_SLICE_U32, NO_ORIGIN);
        let _ = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: leaf_id,
                    link_name: s_leaf,
                },
                function: None,
                args: vec![s],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: rec_id,
                    link_name: s_rec,
                },
                function: None,
                args: vec![s],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        b.finish()
    };

    // `rec(s)` recursively forwards prefix subslice `&s[..2]` (`from: 0`) to `rec` and `s` to `leaf`.
    let rec_fn = {
        let mut b = FuncBuilder::new(rec_id, signature(vec![REF_SLICE_U32], U32));
        let s = b.param(REF_SLICE_U32, NO_ORIGIN);
        let prefix = b.op(
            FuncBuilder::ENTRY,
            OpKind::BorrowView {
                root: Root::Value(s),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 0,
                        to: 2,
                        from_end: false,
                    },
                ],
                mutable: false,
            },
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        let _ = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: rec_id,
                    link_name: s_rec,
                },
                function: None,
                args: vec![prefix],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: leaf_id,
                    link_name: s_leaf,
                },
                function: None,
                args: vec![s],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        b.finish()
    };

    // `exp_rec(s)` is exported and self-recursive: its canonical entry can pass non-zero `off`.
    let exp_rec_fn = {
        let mut b = FuncBuilder::new(exp_rec_id, signature(vec![REF_SLICE_U32], U32));
        let s = b.param(REF_SLICE_U32, NO_ORIGIN);
        let r = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: exp_rec_id,
                    link_name: s_exp_rec,
                },
                function: None,
                args: vec![s],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(r)));
        b.finish()
    };

    // `entry(v: &Vec<u32>, arr: &[u32; 4])`: exercises all positive and negative call patterns.
    let entry_fn = {
        let mut b = FuncBuilder::new(entry_id, signature(vec![REF_VEC_U32, REF_ARR4_U32], U32));
        let v = b.param(REF_VEC_U32, NO_ORIGIN);
        let arr = b.param(REF_ARR4_U32, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let vec_sl = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: vec_as_slice_id,
                    link_name: s_vec_as_slice,
                },
                function: None,
                args: vec![v],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::NativeVecAsSlice),
                host: None,
            },
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        let arr_sl = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::PointerCoercion,
                v: arr,
                from: REF_ARR4_U32,
                to: REF_SLICE_U32,
            },
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        let const_sl = b.op(
            e,
            OpKind::Const(ConstTree::Slice(vec![ConstTree::Scalar {
                bits: 1,
                bytes: 4,
            }])),
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        // Positive calls into `mid` (all zero-offset).
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: mid_id,
                    link_name: s_mid,
                },
                function: None,
                args: vec![vec_sl],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: mid_id,
                    link_name: s_mid,
                },
                function: None,
                args: vec![arr_sl],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: mid_id,
                    link_name: s_mid,
                },
                function: None,
                args: vec![const_sl],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        // Mixed caller: `vec_sl` (off 0) + `&vec_sl[1..3]` (non-zero off) -> `mixed` stays 3-arg.
        let sub_nonzero = b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(vec_sl),
                path: vec![
                    Proj::Deref,
                    Proj::Subslice {
                        from: 1,
                        to: 3,
                        from_end: false,
                    },
                ],
                mutable: false,
            },
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: mixed_id,
                    link_name: s_mixed,
                },
                function: None,
                args: vec![vec_sl],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: mixed_id,
                    link_name: s_mixed,
                },
                function: None,
                args: vec![sub_nonzero],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        // `PtrAdd` (`BinaryOp::Offset`) with offset 1 -> `SliceView` -> `ptr_off` stays 3-arg.
        let base_ptr = b.op(
            e,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: vec_sl,
                from: REF_SLICE_U32,
                to: PTR_U32,
            },
            PTR_U32,
            NO_ORIGIN,
        );
        let one = b.op(
            e,
            OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
            U32,
            NO_ORIGIN,
        );
        let bumped = b.op(
            e,
            OpKind::Binary {
                op: BinaryOp::Offset,
                l: base_ptr,
                r: one,
                ty: PTR_U32,
            },
            PTR_U32,
            NO_ORIGIN,
        );
        let off_view = b.op(
            e,
            OpKind::SliceView {
                ptr: bumped,
                len: one,
            },
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ptr_off_id,
                    link_name: s_ptr_off,
                },
                function: None,
                args: vec![off_view],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        // Exported self-recursive callee called with `vec_sl`.
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: exp_rec_id,
                    link_name: s_exp_rec,
                },
                function: None,
                args: vec![vec_sl],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        // `StaticSlice { start: 0 }` vs `StaticSlice { start: 2 }`.
        let static_ref = GlobalReference::Static {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: 999,
            },
            link_name: s_static_arr,
        };
        let st0 = b.op(
            e,
            OpKind::Const(ConstTree::StaticSlice {
                target: static_ref,
                path: Vec::new(),
                start: 0,
                length: 4,
            }),
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        let st2 = b.op(
            e,
            OpKind::Const(ConstTree::StaticSlice {
                target: static_ref,
                path: Vec::new(),
                start: 2,
                length: 2,
            }),
            REF_SLICE_U32,
            NO_ORIGIN,
        );
        let _ = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: static_ok_id,
                    link_name: s_static_ok,
                },
                function: None,
                args: vec![st0],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        let r = b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: static_bad_id,
                    link_name: s_static_bad,
                },
                function: None,
                args: vec![st2],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            U32,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(r)));
        b.finish()
    };

    let make_link_item = |name: &str, id: ItemId, sig: Signature, exported: bool, refs: &[&str]| {
        let mut it = link_item(name, refs);
        if exported {
            it.linkage = Linkage::exported(name.to_owned());
        }
        let body = Body {
            item: id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: sig,
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            }],
        };
        let hash = crate::jir::summary::semantic_hash(&body, &shared);
        let physical = PhysicalSummary::of(&body, &shared.types, hash);
        it.jir.body = Some(body);
        it.jir.summary = Some(ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: Some(id),
            },
            body_hash: None,
            semantic_hash: Some(hash),
            availability: Availability::Available,
            deps: refs.iter().map(|r| (*r).to_owned()).collect(),
            abi: None,
            physical,
            kind: ItemKind::Fn,
            linkage: it.linkage.clone(),
            order: None,
            dts: false,
            fallback: rcgjs_object::object::BlobRef {
                offset: 0,
                clen: 0,
                ulen: 0,
            },
            body: None,
            origin: None,
            estimated_bytes: 0,
            class: ItemClass::AGGREGATE,
        });
        it.jir.use_shared_interns(shared.clone());
        it
    };

    let _ = s_entry;
    let table = BTreeMap::from([
        (
            JsName::new("leaf".to_owned()),
            make_link_item("leaf", leaf_id, leaf_fn.sig.clone(), false, &[]),
        ),
        (
            JsName::new("mid".to_owned()),
            make_link_item("mid", mid_id, mid_fn.sig.clone(), false, &["leaf", "rec"]),
        ),
        (
            JsName::new("rec".to_owned()),
            make_link_item("rec", rec_id, rec_fn.sig.clone(), false, &["rec", "leaf"]),
        ),
        (
            JsName::new("mixed".to_owned()),
            make_link_item("mixed", mixed_id, mixed_fn.sig.clone(), false, &[]),
        ),
        (
            JsName::new("ptr_off".to_owned()),
            make_link_item("ptr_off", ptr_off_id, ptr_off_fn.sig.clone(), false, &[]),
        ),
        (
            JsName::new("uncalled".to_owned()),
            make_link_item("uncalled", uncalled_id, uncalled_fn.sig.clone(), false, &[]),
        ),
        (
            JsName::new("exp_rec".to_owned()),
            make_link_item(
                "exp_rec",
                exp_rec_id,
                exp_rec_fn.sig.clone(),
                true,
                &["exp_rec"],
            ),
        ),
        (
            JsName::new("static_ok".to_owned()),
            make_link_item(
                "static_ok",
                static_ok_id,
                static_ok_fn.sig.clone(),
                false,
                &[],
            ),
        ),
        (
            JsName::new("static_bad".to_owned()),
            make_link_item(
                "static_bad",
                static_bad_id,
                static_bad_fn.sig.clone(),
                false,
                &[],
            ),
        ),
        (
            JsName::new("dead_inlined".to_owned()),
            make_link_item(
                "dead_inlined",
                dead_inlined_id,
                dead_inlined_fn.sig.clone(),
                false,
                &["leaf"],
            ),
        ),
        (
            JsName::new("entry".to_owned()),
            make_link_item(
                "entry",
                entry_id,
                entry_fn.sig.clone(),
                true,
                &[
                    "mid",
                    "mixed",
                    "ptr_off",
                    "exp_rec",
                    "static_ok",
                    "static_bad",
                    "vec_as_slice",
                    // Stale baseline ref to an already-inlined helper (just like `earcut_linked`
                    // referencing inlined `cure_local_intersections_sl`):
                    "dead_inlined",
                ],
            ),
        ),
    ]);
    let funcs = BTreeMap::from([
        (JsName::new("leaf".to_owned()), Ok(leaf_fn)),
        (JsName::new("mid".to_owned()), Ok(mid_fn)),
        (JsName::new("rec".to_owned()), Ok(rec_fn)),
        (JsName::new("mixed".to_owned()), Ok(mixed_fn)),
        (JsName::new("ptr_off".to_owned()), Ok(ptr_off_fn)),
        (JsName::new("uncalled".to_owned()), Ok(uncalled_fn)),
        (JsName::new("exp_rec".to_owned()), Ok(exp_rec_fn)),
        (JsName::new("static_ok".to_owned()), Ok(static_ok_fn)),
        (JsName::new("static_bad".to_owned()), Ok(static_bad_fn)),
        (JsName::new("dead_inlined".to_owned()), Ok(dead_inlined_fn)),
        (JsName::new("entry".to_owned()), Ok(entry_fn)),
    ]);

    let speed_opts = Opts {
        optimizer: crate::opts::OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseSpeed),
        flat_abi: true,
        ..Opts::default()
    };
    let plan = plan_excluding(&table, &funcs, &speed_opts, &BTreeSet::new());

    // Positive cases: `mid`, `rec`, `leaf`, `static_ok` -> 2-arg zero-offset slices.
    for (label, id) in [
        ("mid", mid_id),
        ("rec", rec_id),
        ("leaf", leaf_id),
        ("static_ok", static_ok_id),
    ] {
        let sig = plan
            .call_signature(id)
            .unwrap_or_else(|| panic!("{label} must be flattened"));
        let p = sig.params[0].as_ref().expect("slice param");
        assert!(p.is_off_zero_slice(), "{label} must be off_zero: {p:?}");
        assert_eq!(p.fields(), 2, "{label} must take 2 JS params");
        assert!(sig.slice_jir_positions().is_empty());
        assert_eq!(sig.off_zero_slice_jir_positions(), vec![0]);
        assert_eq!(sig.recursive_jir_positions().len(), 1);
        assert_eq!(
            plan.decisions
                .get(&JsName::new(label.to_owned()))
                .and_then(|d| d.shape.as_deref()),
            Some("slice(2)"),
            "{label} decision shape must be slice(2)"
        );
    }

    // Negative cases: `mixed`, `ptr_off`, `uncalled`, `exp_rec`, `static_bad` -> stay 3-arg!
    for (label, id) in [
        ("mixed", mixed_id),
        ("ptr_off", ptr_off_id),
        ("uncalled", uncalled_id),
        ("exp_rec", exp_rec_id),
        ("static_bad", static_bad_id),
    ] {
        let sig = plan
            .call_signature(id)
            .unwrap_or_else(|| panic!("{label} must still be 3-arg flattened"));
        let p = sig.params[0].as_ref().expect("slice param");
        assert!(
            !p.is_off_zero_slice(),
            "{label} MUST NOT be off_zero: {p:?}"
        );
        assert_eq!(p.fields(), 3, "{label} must keep 3 JS params");
        assert_eq!(sig.slice_jir_positions(), vec![0]);
        assert!(sig.off_zero_slice_jir_positions().is_empty());
        assert!(sig.recursive_jir_positions().is_empty());
    }

    // ReleaseBalanced keeps canonical 3-arg slice shape (preserves balanced gate invariants).
    let bal_opts = Opts {
        optimizer: crate::opts::OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseBalanced),
        flat_abi: true,
        ..Opts::default()
    };
    let bal_plan = plan_excluding(&table, &funcs, &bal_opts, &BTreeSet::new());
    assert!(!bal_plan.has_off_zero_slice());
    assert_eq!(
        bal_plan.call_signature(leaf_id).expect("leaf").params[0]
            .as_ref()
            .expect("slice")
            .fields(),
        3
    );
}

#[test]
fn structured_cost_key_distinguishes_abi_variants_without_debug_formatting() {
    use crate::plan::physical_cost::HashFeed;

    let digest_of = |plan: &AbiPlan| {
        let mut feed = HashFeed::new();
        plan.extend_cost_key(&mut feed);
        feed.finish()
    };

    let mut plan_a = AbiPlan::default();
    let mut sig_a = FlatSig {
        params: vec![Some(FlatParam::slice(REF_PAIR, vec![REF_PAIR, U32, U32]))],
        result: None,
        spill: 0,
        returns_out_field: false,
    };
    plan_a.flat.insert(item(1), sig_a.clone());
    plan_a.names.insert(item(1), JsName::new("f".to_owned()));

    let mut plan_b = plan_a.clone();
    assert_eq!(digest_of(&plan_a), digest_of(&plan_b));

    sig_a.params[0].as_mut().unwrap().set_off_zero_slice();
    plan_b.flat.insert(item(1), sig_a);
    assert_ne!(digest_of(&plan_a), digest_of(&plan_b));
}

#[test]
fn jir_param_positions_handles_mixed_expanded_and_physical_params() {
    let mut off_zero = FlatParam::slice(REF_PAIR, vec![REF_PAIR, U32, U32]);
    off_zero.set_off_zero_slice();
    let sig = FlatSig {
        params: vec![
            Some(flat_pair()),              // JIR slots 0, 1 (arity 2)
            Some(FlatParam::i64_pair(U32)), // JIR slot 2
            Some(off_zero),                 // JIR slot 3
            Some(FlatParam::byte_str_param(REF_PAIR, U32, ByteStrParam::Full)), // JIR slot 4
            Some(FlatParam::slice(REF_PAIR, vec![REF_PAIR, U32, U32])), // JIR slot 5
        ],
        result: None,
        spill: 0,
        returns_out_field: false,
    };
    assert_eq!(sig.i64_pair_jir_positions(), vec![2]);
    assert_eq!(sig.off_zero_slice_jir_positions(), vec![3]);
    assert_eq!(sig.byte_str_jir_positions(), vec![(4, ByteStrParam::Full)]);
    assert_eq!(sig.slice_jir_positions(), vec![5]);
    assert_eq!(sig.recursive_jir_positions().len(), 1);
    assert_eq!(sig.recursive_jir_positions()[0].0, 3);
}
