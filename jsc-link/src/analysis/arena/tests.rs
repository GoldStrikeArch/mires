use super::*;
use crate::analysis::heap::HeapPlan;
use crate::item::LinkJir;
use crate::jir::repr::FuncBuilder;
use crate::jir::semantic::{AbiSignature, BinaryOp, Body, BodyKind, Signature, UnaryOp};
use crate::jir::types::{
    AllocatorIdentity, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts,
    PointerForm, RepresentationCandidate, TypeDescriptor, TypeMeta, VariantDescriptor,
};
use crate::jir::OriginId;
use rcgjs_object::item::AllocatorProvider;
use rcgjs_object::jir::repr::{RAssertMessage, RTerm};
use rcgjs_object::object::Linkage;

const UNIT: TypeId = TypeId(1);
const U32: TypeId = TypeId(2);
const BOOL: TypeId = TypeId(3);
const NODE: TypeId = TypeId(10);
const VEC_NODE: TypeId = TypeId(11);
const REF_MUT_VEC_NODE: TypeId = TypeId(12);
const SLICE_NODE: TypeId = TypeId(13);
const REF_SLICE_NODE: TypeId = TypeId(14);
const REF_MUT_SLICE_NODE: TypeId = TypeId(15);
const REF_MUT_U32: TypeId = TypeId(16);
const RAW_NODE: TypeId = TypeId(17);
const VEC_U32: TypeId = TypeId(18);
const REF_MUT_VEC_U32: TypeId = TypeId(19);
const NO_ORIGIN: OriginId = OriginId(0);

const NODE_DEF: DefinitionId = DefinitionId {
    crate_id: 99,
    local_hash: 1001,
};
const VEC_DEF: DefinitionId = DefinitionId {
    crate_id: 2,
    local_hash: 2002,
};

fn make_desc(id: TypeId, kind: TypeKind, size: u64, is_indirect: bool) -> TypeDescriptor {
    let int = match kind {
        TypeKind::Integer { signed, bits } => Some(IntFacts {
            signed,
            bits,
            repr: IntRepr::Number,
        }),
        _ => None,
    };
    let sized = !matches!(kind, TypeKind::Slice(_));
    TypeDescriptor {
        id,
        name: None,
        kind,
        layout: LayoutFacts {
            size: Some(size),
            align: Some(size.clamp(1, 8)),
            sized,
            zst: size == 0,
            needs_drop: false,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int,
            is_indirect,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: id,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        },
    }
}

fn make_arena_interns() -> Interns {
    let mut interns = Interns::new();
    let prev_k = interns.intern_str("prev");
    let next_z_k = interns.intern_str("next_z");
    let len_k = interns.intern_str("len");
    let node_k = interns.intern_str("Node");
    let vec_k = interns.intern_str("Vec");
    let _hook = interns.intern_str("std_hook");

    interns
        .types
        .insert(make_desc(UNIT, TypeKind::Tuple(Vec::new()), 0, false));
    interns.types.insert(make_desc(
        U32,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        4,
        false,
    ));
    interns
        .types
        .insert(make_desc(BOOL, TypeKind::Bool, 1, false));
    interns.types.insert(make_desc(
        NODE,
        TypeKind::Adt {
            def: NODE_DEF,
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![VariantDescriptor {
                name: node_k,
                fields: vec![
                    FieldDescriptor {
                        key: FieldKey::Name(prev_k),
                        ty: U32,
                        offset: 0,
                    },
                    FieldDescriptor {
                        key: FieldKey::Name(next_z_k),
                        ty: U32,
                        offset: 4,
                    },
                ],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        8,
        true,
    ));

    let mut vec_node = make_desc(
        VEC_NODE,
        TypeKind::Adt {
            def: VEC_DEF,
            form: NominalForm::Struct,
            args: vec![NODE],
            variants: vec![VariantDescriptor {
                name: vec_k,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(len_k),
                    ty: U32,
                    offset: 0,
                }],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        12,
        true,
    );
    vec_node.meta.candidate = RepresentationCandidate::Vec;
    vec_node.meta.allocator = AllocatorIdentity::Global;
    interns.types.insert(vec_node);

    let mut ref_mut_vec_node = make_desc(
        REF_MUT_VEC_NODE,
        TypeKind::Reference {
            pointee: VEC_NODE,
            mutable: true,
        },
        4,
        false,
    );
    ref_mut_vec_node.js.pointer_form = Some(PointerForm::Thin);
    interns.types.insert(ref_mut_vec_node);

    interns
        .types
        .insert(make_desc(SLICE_NODE, TypeKind::Slice(NODE), 0, false));
    let mut ref_sl = make_desc(
        REF_SLICE_NODE,
        TypeKind::Reference {
            pointee: SLICE_NODE,
            mutable: false,
        },
        8,
        false,
    );
    ref_sl.js.pointer_form = Some(PointerForm::FatSlice);
    interns.types.insert(ref_sl);

    let mut ref_mut_sl = make_desc(
        REF_MUT_SLICE_NODE,
        TypeKind::Reference {
            pointee: SLICE_NODE,
            mutable: true,
        },
        8,
        false,
    );
    ref_mut_sl.js.pointer_form = Some(PointerForm::FatSlice);
    interns.types.insert(ref_mut_sl);

    let mut ref_mut_u32 = make_desc(
        REF_MUT_U32,
        TypeKind::Reference {
            pointee: U32,
            mutable: true,
        },
        4,
        false,
    );
    ref_mut_u32.js.pointer_form = Some(PointerForm::Slot);
    interns.types.insert(ref_mut_u32);

    let mut raw_node = make_desc(
        RAW_NODE,
        TypeKind::RawPointer {
            pointee: NODE,
            mutable: true,
        },
        4,
        false,
    );
    raw_node.js.pointer_form = Some(PointerForm::Slot);
    interns.types.insert(raw_node);

    let mut vec_u32 = make_desc(
        VEC_U32,
        TypeKind::Adt {
            def: VEC_DEF,
            form: NominalForm::Struct,
            args: vec![U32],
            variants: vec![VariantDescriptor {
                name: vec_k,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(len_k),
                    ty: U32,
                    offset: 0,
                }],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        12,
        true,
    );
    vec_u32.meta.candidate = RepresentationCandidate::Vec;
    vec_u32.meta.allocator = AllocatorIdentity::Global;
    interns.types.insert(vec_u32);

    let mut ref_mut_vec_u32 = make_desc(
        REF_MUT_VEC_U32,
        TypeKind::Reference {
            pointee: VEC_U32,
            mutable: true,
        },
        4,
        false,
    );
    ref_mut_vec_u32.js.pointer_form = Some(PointerForm::Thin);
    interns.types.insert(ref_mut_vec_u32);

    interns
}

fn make_link_item(
    name: &str,
    item_id: ItemId,
    root: bool,
    interns: Interns,
    sig: Signature,
) -> LinkItem {
    let mut jir = LinkJir::default();
    jir.use_interns(interns);
    jir.body = Some(Body {
        item: item_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: sig,
        locals: Vec::new(),
        blocks: Vec::new(),
    });
    let mut linkage = Linkage::internal();
    linkage.root = root;
    LinkItem {
        name: JsName::new(name.to_owned()),
        kind: ItemKind::Fn,
        crate_name: "arena_test".to_owned(),
        debug_path: format!("arena_test::{name}"),
        js: String::new(),
        refs: BTreeSet::new(),
        zombies: Vec::new(),
        linkage,
        order: None,
        map: Default::default(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: AllocatorProvider::Default,
        jir,
    }
}

fn std_call_op(
    b: &mut FuncBuilder,
    blk: RBlockId,
    std_op: StandardOperation,
    args: Vec<ValueId>,
    ret_ty: Option<TypeId>,
) -> Option<ValueId> {
    let kind = OpKind::Call {
        callee: Callee::Direct {
            item: ItemId {
                definition: DefinitionId {
                    crate_id: 2,
                    local_hash: 9000,
                },
                instantiation: 0,
            },
            link_name: rcgjs_object::jir::StrId(5),
        },
        function: None,
        args,
        track_caller: None,
        semantic: CallSemantic::Standard(std_op),
        host: None,
    };
    if let Some(ty) = ret_ty {
        Some(b.op(blk, kind, ty, NO_ORIGIN))
    } else {
        b.effect(blk, kind, NO_ORIGIN);
        None
    }
}

#[derive(Clone, Copy)]
enum ArenaMutation {
    Clean,
    VecPop,
    VecTruncate,
    VecClear,
    VecResize,
    UnverifiedPrevStore,
    SubsliceView,
    RawPointerView,
    RootExposesElem,
}

fn run_arena_scenario(mode: ArenaMutation) -> ProgramBoundsFacts {
    let interns = make_arena_interns();
    let entry_id = ItemId {
        definition: DefinitionId {
            crate_id: 99,
            local_hash: 1,
        },
        instantiation: 0,
    };
    let sig = if matches!(mode, ArenaMutation::RootExposesElem) {
        Signature {
            abi: AbiSignature {
                parameters: 2,
                ..AbiSignature::default()
            },
            arguments: vec![U32, REF_SLICE_NODE],
            result: U32,
        }
    } else {
        Signature {
            abi: AbiSignature {
                parameters: 1,
                ..AbiSignature::default()
            },
            arguments: vec![U32],
            result: U32,
        }
    };
    let mut b = FuncBuilder::new(entry_id, sig.clone());
    let entry = FuncBuilder::ENTRY;
    let ext = b.param(U32, NO_ORIGIN);
    if matches!(mode, ArenaMutation::RootExposesElem) {
        let _exposed = b.param(REF_SLICE_NODE, NO_ORIGIN);
    }
    let nodes_cell = b.cell(VEC_NODE, true, NO_ORIGIN);
    b.effect(entry, OpKind::CellAlloc(nodes_cell), NO_ORIGIN);
    let init_vec = std_call_op(
        &mut b,
        entry,
        StandardOperation::VecNew,
        vec![],
        Some(VEC_NODE),
    )
    .expect("vec new");
    b.effect(
        entry,
        OpKind::CellStore {
            cell: nodes_cell,
            v: init_vec,
        },
        NO_ORIGIN,
    );
    let vec_ref = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Cell(nodes_cell),
            path: Vec::new(),
            mutable: true,
        },
        REF_MUT_VEC_NODE,
        NO_ORIGIN,
    );
    let pos = std_call_op(
        &mut b,
        entry,
        StandardOperation::VecLen,
        vec![vec_ref],
        Some(U32),
    )
    .expect("len");
    let sentinel = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar {
            bits: u32::MAX as u128,
            bytes: 4,
        }),
        U32,
        NO_ORIGIN,
    );
    let node = b.op(
        entry,
        OpKind::Aggregate {
            ty: NODE,
            variant: 0,
            active_field: None,
            fields: vec![pos, sentinel],
        },
        NODE,
        NO_ORIGIN,
    );
    std_call_op(
        &mut b,
        entry,
        StandardOperation::VecPush,
        vec![vec_ref, node],
        None,
    );
    let sl = std_call_op(
        &mut b,
        entry,
        StandardOperation::VecAsMutSlice,
        vec![vec_ref],
        Some(REF_MUT_SLICE_NODE),
    )
    .expect("as_mut_slice");
    let sl_len = b.op(
        entry,
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v: sl,
            ty: REF_MUT_SLICE_NODE,
        },
        U32,
        NO_ORIGIN,
    );
    let zero = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        U32,
        NO_ORIGIN,
    );
    let ok0 = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: zero,
            r: sl_len,
            ty: U32,
        },
        BOOL,
        NO_ORIGIN,
    );
    b.effect(
        entry,
        OpKind::Assert {
            cond: ok0,
            expected: true,
            msg: RAssertMessage::BoundsCheck {
                length: sl_len,
                index: zero,
            },
            track_caller: None,
        },
        NO_ORIGIN,
    );
    // Valid write: `sl[0].next_z = pos` (pre-push len of `nodes`, proven `< nodes.len`).
    let next_z_ptr = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(sl),
            path: vec![
                Proj::Deref,
                Proj::Index(zero),
                Proj::Field { field: 1, ty: U32 },
            ],
            mutable: true,
        },
        REF_MUT_U32,
        NO_ORIGIN,
    );
    b.effect(
        entry,
        OpKind::StoreThrough {
            ptr: next_z_ptr,
            v: pos,
        },
        NO_ORIGIN,
    );

    match mode {
        ArenaMutation::Clean | ArenaMutation::RootExposesElem => {}
        ArenaMutation::VecPop => {
            std_call_op(
                &mut b,
                entry,
                StandardOperation::VecPop,
                vec![vec_ref],
                None,
            );
        }
        ArenaMutation::VecTruncate => {
            std_call_op(
                &mut b,
                entry,
                StandardOperation::VecTruncate,
                vec![vec_ref, zero],
                None,
            );
        }
        ArenaMutation::VecClear => {
            std_call_op(
                &mut b,
                entry,
                StandardOperation::VecClear,
                vec![vec_ref],
                None,
            );
        }
        ArenaMutation::VecResize => {
            std_call_op(
                &mut b,
                entry,
                StandardOperation::VecResize,
                vec![vec_ref, zero],
                None,
            );
        }
        ArenaMutation::UnverifiedPrevStore => {
            let prev_ptr = b.op(
                entry,
                OpKind::BorrowView {
                    root: Root::Value(sl),
                    path: vec![
                        Proj::Deref,
                        Proj::Index(zero),
                        Proj::Field { field: 0, ty: U32 },
                    ],
                    mutable: true,
                },
                REF_MUT_U32,
                NO_ORIGIN,
            );
            b.effect(
                entry,
                OpKind::StoreThrough {
                    ptr: prev_ptr,
                    v: ext,
                },
                NO_ORIGIN,
            );
        }
        ArenaMutation::SubsliceView => {
            let _sub = b.op(
                entry,
                OpKind::SliceView { ptr: sl, len: zero },
                REF_SLICE_NODE,
                NO_ORIGIN,
            );
        }
        ArenaMutation::RawPointerView => {
            let _raw = b.op(
                entry,
                OpKind::RawView {
                    root: Root::Value(sl),
                    path: vec![Proj::Deref, Proj::Index(zero)],
                    mutable: true,
                },
                RAW_NODE,
                NO_ORIGIN,
            );
        }
    }

    b.terminate(entry, RTerm::Return(Some(zero)));
    let func = b.finish();

    let name = JsName::new("arena_entry".to_owned());
    let mut table = BTreeMap::new();
    table.insert(
        name.clone(),
        make_link_item("arena_entry", entry_id, true, interns, sig),
    );
    let mut funcs = BTreeMap::new();
    funcs.insert(name.clone(), Ok(func));
    let mut live = BTreeSet::new();
    live.insert(name);
    let opts = Opts::default();
    let heap = HeapPlan::default();
    analyze(&table, &funcs, &BTreeMap::new(), &live, &opts, &heap)
}

#[test]
fn monotone_arena_proves_always_valid_and_sentinel_fields() {
    let facts = run_arena_scenario(ArenaMutation::Clean);
    let inv = facts
        .arenas
        .get(&NODE_DEF)
        .expect("clean monotone arena must be proven");
    assert_eq!(inv.fields.get(&0), Some(&ArenaFieldKind::AlwaysValid));
    assert_eq!(
        inv.fields.get(&1),
        Some(&ArenaFieldKind::ValidOrSentinel {
            sentinel: u32::MAX as u128
        })
    );
}

#[test]
fn adversarial_arena_shrink_escape_subslice_and_raw_disqualify() {
    for mode in [
        ArenaMutation::VecPop,
        ArenaMutation::VecTruncate,
        ArenaMutation::VecClear,
        ArenaMutation::VecResize,
        ArenaMutation::SubsliceView,
        ArenaMutation::RawPointerView,
        ArenaMutation::RootExposesElem,
    ] {
        let facts = run_arena_scenario(mode);
        assert!(
            facts.arenas.is_empty(),
            "adversarial arena mutation must disqualify NODE_DEF"
        );
    }
}

#[test]
fn adversarial_unverified_external_write_kills_only_that_field() {
    let facts = run_arena_scenario(ArenaMutation::UnverifiedPrevStore);
    let inv = facts
        .arenas
        .get(&NODE_DEF)
        .expect("other valid field on NODE_DEF remains sound");
    assert!(
        !inv.fields.contains_key(&0),
        "field 0 (prev) received unchecked external param and must be rejected"
    );
    assert_eq!(
        inv.fields.get(&1),
        Some(&ArenaFieldKind::ValidOrSentinel {
            sentinel: u32::MAX as u128
        })
    );
}

#[test]
fn return_vec_len_proves_one_shl_and_cond_bool_and_rejects_mutated_vec() {
    let interns = make_arena_interns();

    // 1. Clean `fn make_pow2(mb: u32) -> Vec<u32>` returning `NativeVecFromElemZeroed(1 << mb)`.
    let pow2_id = ItemId {
        definition: DefinitionId {
            crate_id: 99,
            local_hash: 20,
        },
        instantiation: 0,
    };
    let pow2_sig = Signature {
        abi: AbiSignature {
            parameters: 1,
            ..AbiSignature::default()
        },
        arguments: vec![U32],
        result: VEC_U32,
    };
    let mut b1 = FuncBuilder::new(pow2_id, pow2_sig.clone());
    let mb1 = b1.param(U32, NO_ORIGIN);
    let one1 = b1.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        U32,
        NO_ORIGIN,
    );
    let shl1 = b1.op(
        FuncBuilder::ENTRY,
        OpKind::Binary {
            op: BinaryOp::Shl,
            l: one1,
            r: mb1,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let v1 = std_call_op(
        &mut b1,
        FuncBuilder::ENTRY,
        StandardOperation::NativeVecFromElemZeroed,
        vec![shl1],
        Some(VEC_U32),
    )
    .expect("zeroed vec");
    b1.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(v1)));

    // 2. Clean conditional `fn h_map_like(mb: u32, r: bool) -> Vec<u32>`.
    let cond_id = ItemId {
        definition: DefinitionId {
            crate_id: 99,
            local_hash: 21,
        },
        instantiation: 0,
    };
    let cond_sig = Signature {
        abi: AbiSignature {
            parameters: 2,
            ..AbiSignature::default()
        },
        arguments: vec![U32, BOOL],
        result: VEC_U32,
    };
    let mut b2 = FuncBuilder::new(cond_id, cond_sig.clone());
    let mb2 = b2.param(U32, NO_ORIGIN);
    let flag = b2.param(BOOL, NO_ORIGIN);
    let b_true = b2.block();
    let b_false = b2.block();
    b2.terminate(
        FuncBuilder::ENTRY,
        RTerm::Switch {
            scrut: flag,
            scrut_ty: BOOL,
            cases: vec![(0, b_false, Vec::new())],
            default: (b_true, Vec::new()),
        },
    );
    let one_t = b2.op(
        b_true,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        U32,
        NO_ORIGIN,
    );
    let shl_t = b2.op(
        b_true,
        OpKind::Binary {
            op: BinaryOp::Shl,
            l: one_t,
            r: mb2,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let vt = std_call_op(
        &mut b2,
        b_true,
        StandardOperation::NativeVecFromElemZeroed,
        vec![shl_t],
        Some(VEC_U32),
    )
    .expect("vt");
    b2.terminate(b_true, RTerm::Return(Some(vt)));
    let vf = std_call_op(
        &mut b2,
        b_false,
        StandardOperation::NativeVecFromElemZeroed,
        vec![mb2],
        Some(VEC_U32),
    )
    .expect("vf");
    b2.terminate(b_false, RTerm::Return(Some(vf)));

    // 3. Adversarial: allocates `1 << mb`, then pushes one more element -> must NOT summarize!
    let bad_id = ItemId {
        definition: DefinitionId {
            crate_id: 99,
            local_hash: 22,
        },
        instantiation: 0,
    };
    let mut b3 = FuncBuilder::new(bad_id, pow2_sig.clone());
    let mb3 = b3.param(U32, NO_ORIGIN);
    let cell3 = b3.cell(VEC_U32, true, NO_ORIGIN);
    b3.effect(FuncBuilder::ENTRY, OpKind::CellAlloc(cell3), NO_ORIGIN);
    let one3 = b3.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        U32,
        NO_ORIGIN,
    );
    let shl3 = b3.op(
        FuncBuilder::ENTRY,
        OpKind::Binary {
            op: BinaryOp::Shl,
            l: one3,
            r: mb3,
            ty: U32,
        },
        U32,
        NO_ORIGIN,
    );
    let v3 = std_call_op(
        &mut b3,
        FuncBuilder::ENTRY,
        StandardOperation::NativeVecFromElemZeroed,
        vec![shl3],
        Some(VEC_U32),
    )
    .expect("v3");
    b3.effect(
        FuncBuilder::ENTRY,
        OpKind::CellStore { cell: cell3, v: v3 },
        NO_ORIGIN,
    );
    let v3_ref = b3.op(
        FuncBuilder::ENTRY,
        OpKind::BorrowView {
            root: Root::Cell(cell3),
            path: Vec::new(),
            mutable: true,
        },
        REF_MUT_VEC_U32,
        NO_ORIGIN,
    );
    std_call_op(
        &mut b3,
        FuncBuilder::ENTRY,
        StandardOperation::VecPush,
        vec![v3_ref, one3],
        None,
    );
    let loaded = b3.op(
        FuncBuilder::ENTRY,
        OpKind::CellLoad(cell3),
        VEC_U32,
        NO_ORIGIN,
    );
    b3.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(loaded)));

    let n1 = JsName::new("make_pow2".to_owned());
    let n2 = JsName::new("h_map_like".to_owned());
    let n3 = JsName::new("make_pow2_mutated".to_owned());
    let mut table = BTreeMap::new();
    table.insert(
        n1.clone(),
        make_link_item(
            "make_pow2",
            pow2_id,
            true,
            interns.clone(),
            pow2_sig.clone(),
        ),
    );
    table.insert(
        n2.clone(),
        make_link_item("h_map_like", cond_id, true, interns.clone(), cond_sig),
    );
    table.insert(
        n3.clone(),
        make_link_item("make_pow2_mutated", bad_id, true, interns, pow2_sig),
    );
    let mut funcs = BTreeMap::new();
    funcs.insert(n1.clone(), Ok(b1.finish()));
    funcs.insert(n2.clone(), Ok(b2.finish()));
    funcs.insert(n3.clone(), Ok(b3.finish()));
    let live: BTreeSet<JsName> = [n1, n2, n3].into_iter().collect();

    let facts = analyze(
        &table,
        &funcs,
        &BTreeMap::new(),
        &live,
        &Opts::default(),
        &HeapPlan::default(),
    );
    assert_eq!(
        facts.return_vec_len.get(&pow2_id),
        Some(&ReturnVecLen::OneShlParam {
            shift_param: 0,
            bits: 32
        })
    );
    assert_eq!(
        facts.return_vec_len.get(&cond_id),
        Some(&ReturnVecLen::CondBoolParam {
            cond_param: 1,
            when_true: Box::new(ReturnVecLen::OneShlParam {
                shift_param: 0,
                bits: 32
            }),
            when_false: Box::new(ReturnVecLen::Param(0)),
        })
    );
    assert!(
        !facts.return_vec_len.contains_key(&bad_id),
        "mutated Vec with extra push must NOT be summarized with initial alloc length"
    );
}
