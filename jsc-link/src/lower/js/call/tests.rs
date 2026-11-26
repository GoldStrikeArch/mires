use std::collections::BTreeSet;

use rcgjs_object::item::{Availability, ItemSummary, JsName, SemanticKey};
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::origin::OriginId;
use rcgjs_object::jir::repr::{
    Carrier, Func, Op, OpKind, RBlock, RBlockId, RTerm, ValueDef, ValueId,
};
use rcgjs_object::jir::semantic::{
    AbiSignature, Body, BodyKind, BodySemantic, CallSemantic, Callee, ConstTree, DefinitionId,
    ItemId, Signature, StandardOperation,
};
use rcgjs_object::jir::types::{
    AllocatorIdentity, EnumRepr, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts,
    LayoutFacts, NominalForm, PointerForm, RepresentationCandidate, TypeDescriptor, TypeId,
    TypeKind, TypeMeta, VariantDescriptor,
};
use rcgjs_object::jir::ItemClass;
use rcgjs_object::jsast;
use rcgjs_object::object::{BlobRef, ItemKind, Linkage};

use crate::analysis::byte_views::ByteViewPlan;
use crate::analysis::heap::HeapPlan;
use crate::lower::js::{host, unsize};
use crate::lower::{LowerInput, NestedClone};
use crate::opts::{Opts, Profile};

fn mk_desc(id: u128, kind: TypeKind, size: u64, is_indirect: bool) -> TypeDescriptor {
    let tid = TypeId(id);
    let int = match kind {
        TypeKind::Integer { signed, bits } => Some(IntFacts {
            signed,
            bits,
            repr: if bits > 32 {
                IntRepr::BigInt
            } else {
                IntRepr::Number
            },
        }),
        _ => None,
    };
    TypeDescriptor {
        id: tid,
        name: None,
        kind,
        layout: LayoutFacts {
            size: Some(size),
            align: Some(size.max(1)),
            sized: true,
            zst: size == 0,
            ..LayoutFacts::default()
        },
        js: JsFacts {
            int,
            is_indirect,
            needs_clone: is_indirect,
            peeled: tid,
            transparent: None,
            crosses_transparent_union: false,
            enum_repr: None,
            is_primitive_enum: false,
            unsized_tail: None,
            byte_element: false,
            pointer_form: None,
            fields: Vec::new(),
        },
        meta: TypeMeta::default(),
    }
}

fn lower_func(interns: &Interns, func: &Func, heap: &HeapPlan) -> String {
    let body = Body {
        item: func.item,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: func.sig.clone(),
        locals: Vec::new(),
        blocks: Vec::new(),
    };
    let summary = ItemSummary {
        key: SemanticKey {
            link_name: "test_fn".to_owned(),
            item: None,
        },
        body_hash: None,
        semantic_hash: None,
        availability: Availability::Available,
        deps: Vec::new(),
        abi: None,
        physical: None,
        kind: ItemKind::Fn,
        linkage: Linkage::internal(),
        order: None,
        dts: false,
        fallback: BlobRef {
            offset: 0,
            clen: 0,
            ulen: 0,
        },
        body: None,
        origin: None,
        estimated_bytes: 0,
        class: ItemClass::default(),
    };
    let js_name = JsName::new("test_fn".to_owned());
    let opts = Opts {
        profile: Some(Profile::ReleaseSpeed),
        ..Opts::default()
    };
    let items = host::ItemIndex::new();
    let vtables = unsize::VtableIndex::new();
    let input = LowerInput {
        name: &js_name,
        summary: &summary,
        body: &body,
        origins: None,
        interns,
        table: &interns.types,
        opts: &opts,
        items: &items,
        drop_glue: host::EMPTY_DROP_GLUE,
        vtables: &vtables,
        spills: &[],
        abi: None,
        final_lowering: false,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
        bigints: None,
        byte_views: ByteViewPlan {
            raw_byte_derefs: false,
            elide_scale_scalar: true,
        },
        heap: Some(heap),
        nested_clone: NestedClone::default(),
    };
    jsast::stmt_to_string(&crate::lower::lower_func(&input, func).expect("lower").decl)
}

#[test]
fn phase1b_lowers_packed_vec_monomorphic_ta_and_fast_vec_resize() {
    let mut interns = Interns::new();
    let sym = interns.intern_str("std_hook");
    let resize_sym = interns.intern_str("alloc$vec$Vec$resize$h1");
    let len_key = interns.intern_str("len");
    let x_key = interns.intern_str("x");
    let none_name = interns.intern_str("None");
    let some_name = interns.intern_str("Some");
    let f0_name = interns.intern_str("_0");

    let unit = TypeId(1);
    let u8_ty = TypeId(2);
    let usize_ty = TypeId(3);
    let obj_ty = TypeId(4);
    interns
        .types
        .insert(mk_desc(1, TypeKind::Tuple(Vec::new()), 0, false));
    interns.types.insert(mk_desc(
        2,
        TypeKind::Integer {
            signed: false,
            bits: 8,
        },
        1,
        false,
    ));
    interns.types.insert(mk_desc(
        3,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        4,
        false,
    ));
    interns.types.insert(mk_desc(
        4,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 5,
                local_hash: 1,
            },
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![VariantDescriptor {
                name: x_key,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(x_key),
                    ty: usize_ty,
                    offset: 0,
                }],
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

    let mk_vec = |interns: &mut Interns, vid: u128, rid: u128, elem: TypeId| {
        let mut vd = mk_desc(
            vid,
            TypeKind::Adt {
                def: DefinitionId {
                    crate_id: 2,
                    local_hash: 1,
                },
                form: NominalForm::Struct,
                args: vec![elem],
                variants: vec![VariantDescriptor {
                    name: len_key,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(len_key),
                        ty: usize_ty,
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
        vd.meta.candidate = RepresentationCandidate::Vec;
        vd.meta.allocator = AllocatorIdentity::Global;
        interns.types.insert(vd);
        let mut rd = mk_desc(
            rid,
            TypeKind::Reference {
                pointee: TypeId(vid),
                mutable: true,
            },
            4,
            false,
        );
        rd.js.pointer_form = Some(PointerForm::Thin);
        interns.types.insert(rd);
        (TypeId(vid), TypeId(rid))
    };

    let (vec_u8, ref_vec_u8) = mk_vec(&mut interns, 10, 11, u8_ty);
    let (vec_obj, ref_vec_obj) = mk_vec(&mut interns, 12, 13, obj_ty);

    let opt_obj = TypeId(14);
    let mut opt_desc = mk_desc(
        14,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 7,
            },
            form: NominalForm::Enum,
            args: vec![obj_ty],
            variants: vec![
                VariantDescriptor {
                    name: none_name,
                    fields: Vec::new(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
                VariantDescriptor {
                    name: some_name,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(f0_name),
                        ty: obj_ty,
                        offset: 0,
                    }],
                    logical_discr: 1,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
            ],
            niche: None,
        },
        8,
        false,
    );
    opt_desc.js.enum_repr = Some(EnumRepr::NullNiche {
        none_variant: 0,
        some_variant: 1,
    });
    interns.types.insert(opt_desc);

    let mut heap = HeapPlan {
        typed_heap_enabled: true,
        native_string_enabled: false,
        native_vec_enabled: true,
        raw_alloc_reachable: false,
        non_rc_raw_alloc_reachable: false,
        any_boxed: false,
        boxed_defs: BTreeSet::new(),
        boxed_non_adt_keys: BTreeSet::new(),
        all_non_adt_boxed: false,
        all_adts_boxed: false,
        smi_u32_fields: BTreeSet::new(),
        packed_arr_vec_types: BTreeSet::from([vec_obj]),
    };

    let std_call = |res, op, args, name| Op {
        result: res,
        origin: OriginId(0),
        kind: OpKind::Call {
            callee: Callee::Direct {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 2,
                        local_hash: 99,
                    },
                    instantiation: 1,
                },
                link_name: name,
            },
            function: None,
            args,
            track_caller: None,
            semantic: CallSemantic::Standard(op),
            host: None,
        },
    };

    // 1. Vec<u8>: with_capacity(n), zeroed(n), push(b), resize(n, b)
    let f_u8 = Func {
        item: ItemId {
            definition: DefinitionId {
                crate_id: 9,
                local_hash: 1,
            },
            instantiation: 1,
        },
        sig: Signature {
            arguments: vec![ref_vec_u8, usize_ty, u8_ty],
            result: unit,
            abi: AbiSignature::default(),
        },
        params: vec![ValueId(0), ValueId(1), ValueId(2)],
        cells: Vec::new(),
        values: vec![
            ValueDef {
                ty: ref_vec_u8,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: usize_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: u8_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: vec_u8,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: vec_u8,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
        ],
        blocks: vec![RBlock {
            id: RBlockId(0),
            params: vec![ValueId(0), ValueId(1), ValueId(2)],
            ops: vec![
                std_call(
                    Some(ValueId(3)),
                    StandardOperation::NativeVecWithCapacity,
                    vec![ValueId(1)],
                    sym,
                ),
                std_call(
                    Some(ValueId(4)),
                    StandardOperation::NativeVecFromElemZeroed,
                    vec![ValueId(1)],
                    sym,
                ),
                std_call(
                    None,
                    StandardOperation::NativeVecPush,
                    vec![ValueId(0), ValueId(2)],
                    sym,
                ),
                std_call(
                    None,
                    StandardOperation::VecResize,
                    vec![ValueId(0), ValueId(1), ValueId(2)],
                    resize_sym,
                ),
            ],
            term: RTerm::Return(None),
        }],
    };
    let js_u8 = lower_func(&interns, &f_u8, &heap);
    assert!(js_u8.contains("__rt.nv_with_cap_u8("), "{js_u8}");
    assert!(js_u8.contains("__rt.nv_zeroed_u8("), "{js_u8}");
    assert!(js_u8.contains("__rt.nv_grow_u8("), "{js_u8}");
    assert!(js_u8.contains("__rt.nv_reserve_u8("), "{js_u8}");
    assert!(js_u8.contains(".buf.fill("), "{js_u8}");
    assert!(!js_u8.contains("nv_grow_one_ta"), "{js_u8}");
    assert!(!js_u8.contains("nv_with_cap_ta"), "{js_u8}");
    assert!(!js_u8.contains("nv_zeroed_ta"), "{js_u8}");
    assert!(!js_u8.contains("alloc$vec$Vec$resize$h1"), "{js_u8}");

    // 2. Packed Vec<Obj>: new, with_capacity(16), push, pop, reserve, shrink_to
    let f_obj = Func {
        item: ItemId {
            definition: DefinitionId {
                crate_id: 9,
                local_hash: 2,
            },
            instantiation: 1,
        },
        sig: Signature {
            arguments: vec![ref_vec_obj, obj_ty, usize_ty],
            result: opt_obj,
            abi: AbiSignature::default(),
        },
        params: vec![ValueId(0), ValueId(1), ValueId(2)],
        cells: Vec::new(),
        values: vec![
            ValueDef {
                ty: ref_vec_obj,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: obj_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: usize_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: usize_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: vec_obj,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: vec_obj,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: opt_obj,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
        ],
        blocks: vec![RBlock {
            id: RBlockId(0),
            params: vec![ValueId(0), ValueId(1), ValueId(2)],
            ops: vec![
                Op {
                    result: Some(ValueId(3)),
                    origin: OriginId(0),
                    kind: OpKind::Const(ConstTree::Scalar { bits: 16, bytes: 4 }),
                },
                std_call(
                    Some(ValueId(4)),
                    StandardOperation::NativeVecNew,
                    vec![],
                    sym,
                ),
                std_call(
                    Some(ValueId(5)),
                    StandardOperation::NativeVecWithCapacity,
                    vec![ValueId(3)],
                    sym,
                ),
                std_call(
                    None,
                    StandardOperation::NativeVecReserve,
                    vec![ValueId(0), ValueId(2), ValueId(3)],
                    sym,
                ),
                std_call(
                    None,
                    StandardOperation::NativeVecShrinkTo,
                    vec![ValueId(0), ValueId(2)],
                    sym,
                ),
                std_call(
                    None,
                    StandardOperation::NativeVecPush,
                    vec![ValueId(0), ValueId(1)],
                    sym,
                ),
                std_call(
                    Some(ValueId(6)),
                    StandardOperation::NativeVecPop,
                    vec![ValueId(0)],
                    sym,
                ),
            ],
            term: RTerm::Return(Some(ValueId(6))),
        }],
    };
    let js_packed = lower_func(&interns, &f_obj, &heap);
    assert!(js_packed.contains("buf: []"), "{js_packed}");
    assert!(js_packed.contains(".push("), "{js_packed}");
    assert!(js_packed.contains(".pop()"), "{js_packed}");
    assert!(!js_packed.contains("new Array("), "{js_packed}");
    assert!(!js_packed.contains("nv_grow_one_arr"), "{js_packed}");
    assert!(!js_packed.contains("nv_reserve_arr"), "{js_packed}");
    assert!(!js_packed.contains("nv_shrink_arr"), "{js_packed}");
    assert!(!js_packed.contains("_nv_ea"), "{js_packed}");

    // 3. Non-TA VecResize (Vec<Obj>) must NOT be intercepted as TypedArray fill
    heap.packed_arr_vec_types.clear();
    let f_obj_resize = Func {
        item: ItemId {
            definition: DefinitionId {
                crate_id: 9,
                local_hash: 3,
            },
            instantiation: 1,
        },
        sig: Signature {
            arguments: vec![ref_vec_obj, usize_ty, obj_ty],
            result: unit,
            abi: AbiSignature::default(),
        },
        params: vec![ValueId(0), ValueId(1), ValueId(2)],
        cells: Vec::new(),
        values: vec![
            ValueDef {
                ty: ref_vec_obj,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: usize_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: obj_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
        ],
        blocks: vec![RBlock {
            id: RBlockId(0),
            params: vec![ValueId(0), ValueId(1), ValueId(2)],
            ops: vec![std_call(
                None,
                StandardOperation::VecResize,
                vec![ValueId(0), ValueId(1), ValueId(2)],
                resize_sym,
            )],
            term: RTerm::Return(None),
        }],
    };
    let js_obj_resize = lower_func(&interns, &f_obj_resize, &heap);
    assert!(
        js_obj_resize.contains("alloc$vec$Vec$resize$h1("),
        "{js_obj_resize}"
    );
}

#[allow(clippy::too_many_arguments)]
fn lower_func_bstr(
    interns: &Interns,
    func: &Func,
    heap: &HeapPlan,
    byte_views: ByteViewPlan,
    items: &host::ItemIndex,
    ascii: Option<&crate::analysis::ascii::AsciiFacts>,
) -> String {
    let body = Body {
        item: func.item,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: func.sig.clone(),
        locals: Vec::new(),
        blocks: Vec::new(),
    };
    let summary = ItemSummary {
        key: SemanticKey {
            link_name: "test_fn".to_owned(),
            item: None,
        },
        body_hash: None,
        semantic_hash: None,
        availability: Availability::Available,
        deps: Vec::new(),
        abi: None,
        physical: None,
        kind: ItemKind::Fn,
        linkage: Linkage::internal(),
        order: None,
        dts: false,
        fallback: BlobRef {
            offset: 0,
            clen: 0,
            ulen: 0,
        },
        body: None,
        origin: None,
        estimated_bytes: 0,
        class: ItemClass::default(),
    };
    let js_name = JsName::new("test_fn".to_owned());
    let opts = Opts {
        profile: Some(Profile::ReleaseSpeed),
        str_repr: crate::opts::StrRepr::Bytes,
        ..Opts::default()
    };
    let vtables = unsize::VtableIndex::new();
    let input = LowerInput {
        name: &js_name,
        summary: &summary,
        body: &body,
        origins: None,
        interns,
        table: &interns.types,
        opts: &opts,
        items,
        drop_glue: host::EMPTY_DROP_GLUE,
        vtables: &vtables,
        spills: &[],
        abi: None,
        final_lowering: false,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
        bigints: None,
        byte_views,
        heap: Some(heap),
        nested_clone: NestedClone::default(),
    };
    let func_ascii = ascii.map(|a| a.for_item(&js_name));
    jsast::stmt_to_string(
        &crate::lower::lower_func_with_ascii(&input, func, func_ascii)
            .expect("lower")
            .decl,
    )
}

#[test]
fn phase2c_known_ascii_str_and_fast_str_lowerings() {
    let mut interns = Interns::new();
    let sym = interns.intern_str("core$str$hook$h0123456789abcdef");
    let start_key = interns.intern_str("start");
    let none_name = interns.intern_str("None");
    let some_name = interns.intern_str("Some");
    let f0_name = interns.intern_str("_0");
    let lit_lt = interns.intern_str("<");
    let lit_gt = interns.intern_str(">");
    let lit_empty = interns.intern_str("");
    let lit_pat = interns.intern_str("ab");

    let unit = TypeId(1);
    let bool_ty = TypeId(2);
    let usize_ty = TypeId(3);
    let char_ty = TypeId(4);
    let str_ty = TypeId(5);
    let ref_str_ty = TypeId(6);
    let range_from_ty = TypeId(7);
    let opt_str_ty = TypeId(8);
    let string_ty = TypeId(9);

    interns
        .types
        .insert(mk_desc(1, TypeKind::Tuple(Vec::new()), 0, false));
    interns.types.insert(mk_desc(2, TypeKind::Bool, 1, false));
    interns.types.insert(mk_desc(
        3,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        4,
        false,
    ));
    interns.types.insert(mk_desc(4, TypeKind::Char, 4, false));
    let mut str_desc = mk_desc(5, TypeKind::Str, 0, false);
    str_desc.layout.sized = false;
    str_desc.layout.size = None;
    str_desc.layout.zst = false;
    interns.types.insert(str_desc);
    let mut ref_str_desc = mk_desc(
        6,
        TypeKind::Reference {
            pointee: str_ty,
            mutable: false,
        },
        8,
        false,
    );
    ref_str_desc.js.pointer_form = Some(PointerForm::Str);
    interns.types.insert(ref_str_desc);

    interns.types.insert(mk_desc(
        7,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 70,
            },
            form: NominalForm::Struct,
            args: vec![usize_ty],
            variants: vec![VariantDescriptor {
                name: start_key,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(start_key),
                    ty: usize_ty,
                    offset: 0,
                }],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        4,
        false,
    ));

    let mut opt_desc = mk_desc(
        8,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 80,
            },
            form: NominalForm::Enum,
            args: vec![ref_str_ty],
            variants: vec![
                VariantDescriptor {
                    name: none_name,
                    fields: Vec::new(),
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
                VariantDescriptor {
                    name: some_name,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(f0_name),
                        ty: ref_str_ty,
                        offset: 0,
                    }],
                    logical_discr: 1,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
            ],
            niche: None,
        },
        8,
        false,
    );
    opt_desc.js.enum_repr = Some(EnumRepr::NullNiche {
        none_variant: 0,
        some_variant: 1,
    });
    interns.types.insert(opt_desc);

    let mut string_desc = mk_desc(
        9,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 2,
                local_hash: 90,
            },
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: Vec::new(),
            niche: None,
        },
        12,
        true,
    );
    string_desc.meta.candidate = RepresentationCandidate::String;
    interns.types.insert(string_desc);

    let mut ascii = crate::analysis::ascii::AsciiFacts::default();
    ascii.funcs.insert(
        JsName::new("test_fn".to_owned()),
        BTreeSet::from([ValueId(0)]),
    );

    let heap = HeapPlan {
        typed_heap_enabled: true,
        native_string_enabled: true,
        native_vec_enabled: true,
        ..HeapPlan::default()
    };

    let std_call = |res, op, args| Op {
        result: res,
        origin: OriginId(0),
        kind: OpKind::Call {
            callee: Callee::Direct {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 50,
                    },
                    instantiation: 1,
                },
                link_name: sym,
            },
            function: None,
            args,
            track_caller: None,
            semantic: CallSemantic::Standard(op),
            host: None,
        },
    };

    // Params: v0 = known-ASCII &str, v1 = unknown &str, v2 = usize, v3 = RangeFrom<usize>.
    let mut values = vec![
        ValueDef {
            ty: ref_str_ty,
            carrier: Carrier::Baseline,
            origin: OriginId(0),
        },
        ValueDef {
            ty: ref_str_ty,
            carrier: Carrier::Baseline,
            origin: OriginId(0),
        },
        ValueDef {
            ty: usize_ty,
            carrier: Carrier::Baseline,
            origin: OriginId(0),
        },
        ValueDef {
            ty: range_from_ty,
            carrier: Carrier::Baseline,
            origin: OriginId(0),
        },
    ];
    let mut ops = Vec::new();
    let mut push_val = |ty| {
        let id = ValueId(values.len() as u32);
        values.push(ValueDef {
            ty,
            carrier: Carrier::Baseline,
            origin: OriginId(0),
        });
        id
    };
    let c_lt = push_val(ref_str_ty);
    ops.push(Op {
        result: Some(c_lt),
        origin: OriginId(0),
        kind: OpKind::Const(ConstTree::Str(lit_lt)),
    });
    let c_gt = push_val(ref_str_ty);
    ops.push(Op {
        result: Some(c_gt),
        origin: OriginId(0),
        kind: OpKind::Const(ConstTree::Str(lit_gt)),
    });
    let c_empty = push_val(ref_str_ty);
    ops.push(Op {
        result: Some(c_empty),
        origin: OriginId(0),
        kind: OpKind::Const(ConstTree::Str(lit_empty)),
    });
    let c_pat = push_val(ref_str_ty);
    ops.push(Op {
        result: Some(c_pat),
        origin: OriginId(0),
        kind: OpKind::Const(ConstTree::Str(lit_pat)),
    });
    let c_nl = push_val(char_ty);
    ops.push(Op {
        result: Some(c_nl),
        origin: OriginId(0),
        kind: OpKind::Const(ConstTree::Scalar { bits: 10, bytes: 4 }),
    });
    let c_eacute = push_val(char_ty);
    ops.push(Op {
        result: Some(c_eacute),
        origin: OriginId(0),
        kind: OpKind::Const(ConstTree::Scalar {
            bits: 0xe9,
            bytes: 4,
        }),
    });
    let c_zero = push_val(usize_ty);
    ops.push(Op {
        result: Some(c_zero),
        origin: OriginId(0),
        kind: OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
    });

    let r_ascii_known = push_val(bool_ty);
    ops.push(std_call(
        Some(r_ascii_known),
        StandardOperation::StrIsAscii,
        vec![ValueId(0)],
    ));
    let r_ascii_unk = push_val(bool_ty);
    ops.push(std_call(
        Some(r_ascii_unk),
        StandardOperation::StrIsAscii,
        vec![ValueId(1)],
    ));
    let r_bound_known = push_val(bool_ty);
    ops.push(std_call(
        Some(r_bound_known),
        StandardOperation::StrIsCharBoundary,
        vec![ValueId(0), ValueId(2)],
    ));
    let r_get_known = push_val(opt_str_ty);
    ops.push(std_call(
        Some(r_get_known),
        StandardOperation::StrGet,
        vec![ValueId(0), ValueId(3)],
    ));
    let r_lower_known = push_val(string_ty);
    ops.push(std_call(
        Some(r_lower_known),
        StandardOperation::StrToLowercase,
        vec![ValueId(0)],
    ));
    let r_sw1 = push_val(bool_ty);
    ops.push(std_call(
        Some(r_sw1),
        StandardOperation::StrStartsWith,
        vec![ValueId(1), c_lt],
    ));
    let r_ew1 = push_val(bool_ty);
    ops.push(std_call(
        Some(r_ew1),
        StandardOperation::BstrEndsWith,
        vec![ValueId(1), c_gt],
    ));
    let r_sw_empty = push_val(bool_ty);
    ops.push(std_call(
        Some(r_sw_empty),
        StandardOperation::BstrStartsWith,
        vec![ValueId(1), c_empty],
    ));
    let r_find_ch = push_val(usize_ty);
    ops.push(std_call(
        Some(r_find_ch),
        StandardOperation::BstrFindChar,
        vec![ValueId(1), c_zero, ValueId(2), c_nl],
    ));
    let r_rfind_ch = push_val(usize_ty);
    ops.push(std_call(
        Some(r_rfind_ch),
        StandardOperation::BstrRfindChar,
        vec![ValueId(1), c_zero, ValueId(2), c_eacute],
    ));
    let r_find_str = push_val(usize_ty);
    ops.push(std_call(
        Some(r_find_str),
        StandardOperation::BstrFindStr,
        vec![ValueId(1), ValueId(2), ValueId(2), c_pat],
    ));

    let func = Func {
        item: ItemId {
            definition: DefinitionId {
                crate_id: 9,
                local_hash: 10,
            },
            instantiation: 1,
        },
        sig: Signature {
            arguments: vec![ref_str_ty, ref_str_ty, usize_ty, range_from_ty],
            result: unit,
            abi: AbiSignature::default(),
        },
        params: vec![ValueId(0), ValueId(1), ValueId(2), ValueId(3)],
        cells: Vec::new(),
        values,
        blocks: vec![RBlock {
            id: RBlockId(0),
            params: vec![ValueId(0), ValueId(1), ValueId(2), ValueId(3)],
            ops,
            term: RTerm::Return(None),
        }],
    };
    let items = host::ItemIndex::new();
    let js = lower_func_bstr(
        &interns,
        &func,
        &heap,
        ByteViewPlan {
            raw_byte_derefs: false,
            elide_scale_scalar: true,
        },
        &items,
        Some(&ascii),
    );
    // KnownAscii elides StrIsAscii, StrIsCharBoundary, StrGet, StrToLowercase on v0.
    assert!(js.contains("__rt._bstr_is_ascii("), "{js}");
    assert!(!js.contains("bstr_boundary"), "{js}");
    assert!(!js.contains("bstr_get_from"), "{js}");
    assert!(!js.contains("bstr_to_lowercase"), "{js}");
    assert!(js.contains(".toLowerCase()"), "{js}");
    // 1-char startsWith/endsWith -> charCodeAt(0) === 60 / charCodeAt(len - 1) === 62.
    assert!(js.contains(".charCodeAt(0) === 60"), "{js}");
    assert!(js.contains("length - 1) === 62"), "{js}");
    // Constant BstrFindChar / BstrRfindChar / BstrFindStr -> inline indexOf / lastIndexOf.
    assert!(!js.contains("bstr_find_char"), "{js}");
    assert!(!js.contains("bstr_rfind_char"), "{js}");
    assert!(!js.contains("bstr_find_str"), "{js}");
    assert!(js.contains(".indexOf(\"\\n\""), "{js}");
    assert!(js.contains(".lastIndexOf(\"Ã©\""), "{js}");
    assert!(js.contains(".indexOf(\"ab\""), "{js}");
}

#[test]
fn phase2c_bstr_slice_search_and_slice_reverse_lowerings() {
    let mut interns = Interns::new();
    let rev_sym = interns.intern_str("core$slice$impl_0$reverse$h0123456789abcdef");
    let spoof_sym = interns.intern_str("user$slice$reverse$h0123456789abcdef");
    let x_key = interns.intern_str("x");

    let unit = TypeId(1);
    let bool_ty = TypeId(2);
    let u8_ty = TypeId(3);
    let u32_ty = TypeId(4);
    let str_ty = TypeId(5);
    let ref_slice_u8_ty = TypeId(8);
    let mut_slice_u32_ty = TypeId(10);
    let struct_ty = TypeId(11);
    let mut_slice_struct_ty = TypeId(13);
    let mut_slice_unit_ty = TypeId(15);

    interns
        .types
        .insert(mk_desc(1, TypeKind::Tuple(Vec::new()), 0, false));
    interns.types.insert(mk_desc(2, TypeKind::Bool, 1, false));
    interns.types.insert(mk_desc(
        3,
        TypeKind::Integer {
            signed: false,
            bits: 8,
        },
        1,
        false,
    ));
    interns.types.insert(mk_desc(
        4,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        4,
        false,
    ));
    let mut str_desc = mk_desc(5, TypeKind::Str, 0, false);
    str_desc.layout.sized = false;
    str_desc.layout.size = None;
    str_desc.layout.zst = false;
    interns.types.insert(str_desc);
    let mut ref_str_desc = mk_desc(
        6,
        TypeKind::Reference {
            pointee: str_ty,
            mutable: false,
        },
        8,
        false,
    );
    ref_str_desc.js.pointer_form = Some(PointerForm::Str);
    interns.types.insert(ref_str_desc);

    let mk_slice_ref = |interns: &mut Interns, sid: u128, rid: u128, elem: TypeId, mut_: bool| {
        let mut sd = mk_desc(sid, TypeKind::Slice(elem), 0, false);
        sd.layout.sized = false;
        sd.layout.size = None;
        sd.layout.zst = false;
        interns.types.insert(sd);
        let mut rd = mk_desc(
            rid,
            TypeKind::Reference {
                pointee: TypeId(sid),
                mutable: mut_,
            },
            8,
            false,
        );
        rd.js.pointer_form = Some(PointerForm::FatSlice);
        interns.types.insert(rd);
    };
    mk_slice_ref(&mut interns, 7, 8, u8_ty, false);
    mk_slice_ref(&mut interns, 9, 10, u32_ty, true);
    interns.types.insert(mk_desc(
        11,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 5,
                local_hash: 11,
            },
            form: NominalForm::Struct,
            args: Vec::new(),
            variants: vec![VariantDescriptor {
                name: x_key,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(x_key),
                    ty: u32_ty,
                    offset: 0,
                }],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        4,
        true,
    ));
    mk_slice_ref(&mut interns, 12, 13, struct_ty, true);
    mk_slice_ref(&mut interns, 14, 15, unit, true);

    let rev_item = ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: 1,
        },
        instantiation: 1,
    };
    let rev_struct_item = ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: 1,
        },
        instantiation: 2,
    };
    let rev_zst_item = ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: 1,
        },
        instantiation: 3,
    };
    let spoof_item = ItemId {
        definition: DefinitionId {
            crate_id: 99,
            local_hash: 1,
        },
        instantiation: 1,
    };
    let sw_item = ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: 2,
        },
        instantiation: 1,
    };
    let ew_item = ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: 3,
        },
        instantiation: 1,
    };

    let mut ascii = crate::analysis::ascii::AsciiFacts::default();
    ascii.slice_reverse.insert(rev_item);
    ascii.slice_reverse.insert(rev_struct_item);
    ascii.slice_reverse.insert(rev_zst_item);
    ascii.slice_starts_with.insert(sw_item);
    ascii.slice_ends_with.insert(ew_item);

    let mut heap = HeapPlan {
        typed_heap_enabled: true,
        native_string_enabled: true,
        native_vec_enabled: true,
        raw_alloc_reachable: false,
        non_rc_raw_alloc_reachable: false,
        any_boxed: false,
        ..HeapPlan::default()
    };
    let items = host::ItemIndex::new();
    let bv_clean = ByteViewPlan {
        raw_byte_derefs: false,
        elide_scale_scalar: true,
    };

    // 1. Primitive &mut [u32] reverse -> inline while loop, no call.
    let mk_rev_func = |slice_ty: TypeId, callee_item: ItemId, link_name| Func {
        item: ItemId {
            definition: DefinitionId {
                crate_id: 9,
                local_hash: 20,
            },
            instantiation: 1,
        },
        sig: Signature {
            arguments: vec![slice_ty],
            result: unit,
            abi: AbiSignature::default(),
        },
        params: vec![ValueId(0)],
        cells: Vec::new(),
        values: vec![ValueDef {
            ty: slice_ty,
            carrier: Carrier::Baseline,
            origin: OriginId(0),
        }],
        blocks: vec![RBlock {
            id: RBlockId(0),
            params: vec![ValueId(0)],
            ops: vec![Op {
                result: None,
                origin: OriginId(0),
                kind: OpKind::Call {
                    callee: Callee::Direct {
                        item: callee_item,
                        link_name,
                    },
                    function: None,
                    args: vec![ValueId(0)],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
            }],
            term: RTerm::Return(None),
        }],
    };

    let js_u32 = lower_func_bstr(
        &interns,
        &mk_rev_func(mut_slice_u32_ty, rev_item, rev_sym),
        &heap,
        bv_clean,
        &items,
        Some(&ascii),
    );
    assert!(js_u32.contains("while ("), "{js_u32}");
    assert!(!js_u32.contains("core$slice$impl_0$reverse"), "{js_u32}");

    // 2. Indirect untracked &mut [Struct] reverse -> inline while loop.
    let js_struct = lower_func_bstr(
        &interns,
        &mk_rev_func(mut_slice_struct_ty, rev_struct_item, rev_sym),
        &heap,
        bv_clean,
        &items,
        Some(&ascii),
    );
    assert!(js_struct.contains("while ("), "{js_struct}");
    assert!(
        !js_struct.contains("core$slice$impl_0$reverse"),
        "{js_struct}"
    );

    // 3. Indirect tracked (&mut [Struct] with needs_box_tracking) -> MUST fall back!
    heap.any_boxed = true;
    heap.all_adts_boxed = true;
    let js_struct_tracked = lower_func_bstr(
        &interns,
        &mk_rev_func(mut_slice_struct_ty, rev_struct_item, rev_sym),
        &heap,
        bv_clean,
        &items,
        Some(&ascii),
    );
    assert!(
        js_struct_tracked.contains("core$slice$impl_0$reverse"),
        "{js_struct_tracked}"
    );
    heap.any_boxed = false;
    heap.all_adts_boxed = false;

    // 4. raw_byte_derefs == true -> MUST fall back!
    let js_raw_bv = lower_func_bstr(
        &interns,
        &mk_rev_func(mut_slice_u32_ty, rev_item, rev_sym),
        &heap,
        ByteViewPlan {
            raw_byte_derefs: true,
            elide_scale_scalar: false,
        },
        &items,
        Some(&ascii),
    );
    assert!(
        js_raw_bv.contains("core$slice$impl_0$reverse"),
        "{js_raw_bv}"
    );

    // 5. ZST &mut [()] reverse -> elided completely (no while, no call).
    let js_zst = lower_func_bstr(
        &interns,
        &mk_rev_func(mut_slice_unit_ty, rev_zst_item, rev_sym),
        &heap,
        bv_clean,
        &items,
        Some(&ascii),
    );
    assert!(!js_zst.contains("while ("), "{js_zst}");
    assert!(!js_zst.contains("core$slice$impl_0$reverse"), "{js_zst}");

    // 6. Unauthenticated spoof -> MUST call canonical function.
    let js_spoof = lower_func_bstr(
        &interns,
        &mk_rev_func(mut_slice_u32_ty, spoof_item, spoof_sym),
        &heap,
        bv_clean,
        &items,
        Some(&ascii),
    );
    assert!(js_spoof.contains("user$slice$reverse"), "{js_spoof}");

    // 7. Native ByteStr `core::slice::<impl [u8]>::{starts_with,ends_with}` over ByteViews
    // (FullSlice + VarSlice with const 0/1/multi-byte and dynamic ByteView needles).
    let f_bstr_slice = Func {
        item: ItemId {
            definition: DefinitionId {
                crate_id: 9,
                local_hash: 30,
            },
            instantiation: 1,
        },
        sig: Signature {
            arguments: vec![ref_slice_u8_ty, ref_slice_u8_ty],
            result: bool_ty,
            abi: AbiSignature::default(),
        },
        params: vec![ValueId(0), ValueId(1)],
        cells: Vec::new(),
        values: vec![
            ValueDef {
                ty: ref_slice_u8_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: ref_slice_u8_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: ref_slice_u8_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: ref_slice_u8_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
            ValueDef {
                ty: ref_slice_u8_ty,
                carrier: Carrier::Baseline,
                origin: OriginId(0),
            },
        ],
        blocks: vec![RBlock {
            id: RBlockId(0),
            params: vec![ValueId(0), ValueId(1)],
            ops: vec![
                Op {
                    result: Some(ValueId(2)),
                    origin: OriginId(0),
                    kind: OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Slice(vec![
                        ConstTree::Scalar { bits: 96, bytes: 1 },
                        ConstTree::Scalar { bits: 96, bytes: 1 },
                        ConstTree::Scalar { bits: 96, bytes: 1 },
                    ])))),
                },
                Op {
                    result: Some(ValueId(3)),
                    origin: OriginId(0),
                    kind: OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Slice(vec![
                        ConstTree::Scalar { bits: 62, bytes: 1 },
                    ])))),
                },
                Op {
                    result: Some(ValueId(4)),
                    origin: OriginId(0),
                    kind: OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Slice(
                        Vec::new(),
                    )))),
                },
            ],
            term: RTerm::Return(None),
        }],
    };
    let opts = Opts {
        profile: Some(Profile::ReleaseSpeed),
        str_repr: crate::opts::StrRepr::Bytes,
        ..Opts::default()
    };
    let mut plan = crate::lower::plan::Plan::default();
    plan.names.insert(ValueId(0), "h".to_owned());
    plan.names.insert(ValueId(1), "n".to_owned());
    let summary = ItemSummary {
        key: SemanticKey {
            link_name: "test_fn".to_owned(),
            item: None,
        },
        body_hash: None,
        semantic_hash: None,
        availability: Availability::Available,
        deps: Vec::new(),
        abi: None,
        physical: None,
        kind: ItemKind::Fn,
        linkage: Linkage::internal(),
        order: None,
        dts: false,
        fallback: BlobRef {
            offset: 0,
            clen: 0,
            ulen: 0,
        },
        body: None,
        origin: None,
        estimated_bytes: 0,
        class: ItemClass::default(),
    };
    let cx = super::super::JsCx::with_plans(
        &f_bstr_slice,
        &plan,
        &interns.types,
        &interns,
        &opts,
        &summary,
        super::super::ProgramPlans {
            byte_views: bv_clean,
            heap: Some(&heap),
            ascii: Some(ascii.for_item(&JsName::new("test_fn".to_owned()))),
            ..super::super::ProgramPlans::default()
        },
    );

    let full_bv = super::super::ByteView {
        mode: super::super::ByteViewMode::FullSlice,
        base_str: None,
        base_is_string_obj: false,
        borrow_slice: None,
    };
    let var_bv = super::super::ByteView {
        mode: super::super::ByteViewMode::VarSlice,
        base_str: None,
        base_is_string_obj: false,
        borrow_slice: None,
    };

    let render_lowered = |l: super::Lowered| match l {
        super::Lowered::Value(mut stmts, expr) => {
            stmts.push(jsast::ret(expr));
            jsast::program_to_string(&stmts)
        }
        _ => panic!("expected Value"),
    };

    // FullSlice + multi-byte const `b"```"` -> `h.startsWith("```")`.
    let js_full_sw = render_lowered(
        cx.lower_bstr_slice_search_views(true, ValueId(0), full_bv, ValueId(2), None)
            .unwrap()
            .unwrap(),
    );
    assert!(js_full_sw.contains("h.startsWith(\"```\")"), "{js_full_sw}");

    // FullSlice + 1-byte const `b">"` ends_with -> `h.charCodeAt(h.length - 1) === 62`.
    let js_full_ew1 = render_lowered(
        cx.lower_bstr_slice_search_views(false, ValueId(0), full_bv, ValueId(3), None)
            .unwrap()
            .unwrap(),
    );
    assert!(
        js_full_ew1.contains("h.charCodeAt(h.length - 1) === 62"),
        "{js_full_ew1}"
    );

    // FullSlice + empty const `b""` -> `true`.
    let js_empty = render_lowered(
        cx.lower_bstr_slice_search_views(true, ValueId(0), full_bv, ValueId(4), None)
            .unwrap()
            .unwrap(),
    );
    assert!(js_empty.contains("return true;"), "{js_empty}");

    // VarSlice + multi-byte const `b"```"` starts_with -> `len >= 3 && buf.startsWith("```", off)`.
    let js_var_sw = render_lowered(
        cx.lower_bstr_slice_search_views(true, ValueId(0), var_bv, ValueId(2), None)
            .unwrap()
            .unwrap(),
    );
    assert!(js_var_sw.contains(">= 3 &&"), "{js_var_sw}");
    assert!(js_var_sw.contains(".startsWith(\"```\","), "{js_var_sw}");

    // VarSlice + multi-byte const `b"```"` ends_with -> `len >= 3 && buf.endsWith("```", off + len)`.
    let js_var_ew = render_lowered(
        cx.lower_bstr_slice_search_views(false, ValueId(0), var_bv, ValueId(2), None)
            .unwrap()
            .unwrap(),
    );
    assert!(js_var_ew.contains(">= 3 &&"), "{js_var_ew}");
    assert!(js_var_ew.contains(".endsWith(\"```\","), "{js_var_ew}");

    // VarSlice + dynamic FullSlice needle -> `len >= n.length && buf.startsWith(n, off)`.
    let js_var_dyn = render_lowered(
        cx.lower_bstr_slice_search_views(true, ValueId(0), var_bv, ValueId(1), Some(full_bv))
            .unwrap()
            .unwrap(),
    );
    assert!(js_var_dyn.contains(">= n.length &&"), "{js_var_dyn}");
    assert!(js_var_dyn.contains(".startsWith(n,"), "{js_var_dyn}");
}
