//! Schema tests: a hand-built body round-trips through postcard, verifies, and the type-table
//! helpers answer the way v1's `value.rs` did.

use super::interns::Interns;
use super::origin::{Origin, OriginId, OriginKind, OriginTable, SourceFact};
use super::semantic::*;
use super::types::*;

fn scalar(id: u128, kind: TypeKind, size: u64) -> TypeDescriptor {
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
        id: TypeId(id),
        name: None,
        kind,
        layout: LayoutFacts {
            size: Some(size),
            unsized_prefix: None,
            align: Some(size.max(1)),
            sized: true,
            zst: size == 0,
            one_zst: size == 0,
            needs_drop: false,
            has_own_drop: false,
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int,
            is_indirect: size == 0,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled: TypeId(id),
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: matches!(kind_bits(id), 8),
            fields: Vec::new(),
        },
    }
}

fn kind_bits(id: u128) -> u32 {
    match id {
        2 => 8,
        _ => 0,
    }
}

const UNIT: TypeId = TypeId(1);
const U8: TypeId = TypeId(2);
const I32: TypeId = TypeId(3);
const OPTION_I32: TypeId = TypeId(4);
const DYN: TypeId = TypeId(5);
const REF_I32: TypeId = TypeId(6);
const NONZERO_I32: TypeId = TypeId(7);
const CORO: TypeId = TypeId(8);

/// A small world: `()`, `u8`, `i32`, `Option<i32>`, `dyn Trait`, `&i32`, `NonZero<i32>`, and a
/// two-state coroutine.
fn build_interns() -> Interns {
    let mut interns = Interns::new();
    let none = interns.intern_str("None");
    let some = interns.intern_str("Some");
    let s0 = interns.intern_str("$s0");
    let s1 = interns.intern_str("$s1");
    let empty = interns.intern_str("");
    let def = |n: u64| DefinitionId {
        crate_id: 1,
        local_hash: n,
    };

    interns.intern_type(scalar(1, TypeKind::Tuple(Vec::new()), 0));
    interns.intern_type(scalar(
        2,
        TypeKind::Integer {
            signed: false,
            bits: 8,
        },
        1,
    ));
    interns.intern_type(scalar(
        3,
        TypeKind::Integer {
            signed: true,
            bits: 32,
        },
        4,
    ));

    let mut option = scalar(4, TypeKind::Opaque, 8);
    option.kind = TypeKind::Adt {
        def: def(10),
        args: vec![I32],
        form: NominalForm::Enum,
        variants: vec![
            VariantDescriptor {
                name: none,
                fields: Vec::new(),
                logical_discr: 0,
                physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 4 }),
                result_ok: false,
                saved_local_keys: Vec::new(),
            },
            VariantDescriptor {
                name: some,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(interns.intern_str("_0")),
                    ty: I32,
                    offset: 4,
                }],
                logical_discr: 1,
                physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 4 }),
                result_ok: false,
                saved_local_keys: Vec::new(),
            },
        ],
        niche: None,
    };
    option.js.is_indirect = true;
    option.js.needs_clone = true;
    option.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(option);

    let mut dynamic = scalar(5, TypeKind::Opaque, 0);
    dynamic.kind = TypeKind::Dynamic {
        principal: Some(def(20)),
        auto: Vec::new(),
    };
    dynamic.layout.sized = false;
    dynamic.layout.size = None;
    dynamic.js.is_indirect = true;
    interns.intern_type(dynamic);

    let mut reference = scalar(6, TypeKind::Opaque, 4);
    reference.kind = TypeKind::Reference {
        pointee: I32,
        mutable: false,
    };
    reference.js.pointer_form = Some(PointerForm::Slot);
    interns.intern_type(reference);

    let mut nonzero = scalar(7, TypeKind::Opaque, 4);
    nonzero.kind = TypeKind::Adt {
        def: def(30),
        args: vec![I32],
        form: NominalForm::Struct,
        variants: vec![VariantDescriptor {
            name: empty,
            fields: vec![FieldDescriptor {
                key: FieldKey::Transparent,
                ty: I32,
                offset: 0,
            }],
            logical_discr: 0,
            physical_tag: None,
            result_ok: false,
            saved_local_keys: Vec::new(),
        }],
        niche: None,
    };
    nonzero.meta.abi = NominalAbi::Transparent;
    nonzero.js.transparent = Some(TransparentField { field: 0, ty: I32 });
    nonzero.js.peeled = I32;
    nonzero.js.int = Some(IntFacts {
        signed: true,
        bits: 32,
        repr: IntRepr::Number,
    });
    nonzero.js.fields = vec![FieldDescriptor {
        key: FieldKey::Transparent,
        ty: I32,
        offset: 0,
    }];
    interns.intern_type(nonzero);

    let mut coroutine = scalar(8, TypeKind::Opaque, 16);
    coroutine.kind = TypeKind::Coroutine {
        def: def(40),
        upvars: vec![I32],
        saved_locals: vec![(0, I32), (1, U8)],
        states: vec![vec![], vec![0, 1]],
        variants: vec![
            VariantDescriptor {
                name: empty,
                fields: Vec::new(),
                logical_discr: 0,
                physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                result_ok: false,
                saved_local_keys: Vec::new(),
            },
            VariantDescriptor {
                name: empty,
                fields: vec![
                    FieldDescriptor {
                        key: FieldKey::Name(s0),
                        ty: I32,
                        offset: 4,
                    },
                    FieldDescriptor {
                        key: FieldKey::Name(s1),
                        ty: U8,
                        offset: 8,
                    },
                ],
                logical_discr: 1,
                physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
                result_ok: false,
                saved_local_keys: vec![s0, s1],
            },
        ],
    };
    coroutine.js.is_indirect = true;
    coroutine.js.needs_clone = true;
    coroutine.js.enum_repr = Some(EnumRepr::State);
    coroutine.js.fields = vec![FieldDescriptor {
        key: FieldKey::Index(0),
        ty: I32,
        offset: 0,
    }];
    interns.intern_type(coroutine);
    interns
}

fn place(local: u32, ty: TypeId) -> Place {
    Place {
        local: LocalId(local),
        projection: Vec::new(),
        ty,
    }
}

fn constant_i32(value: u128) -> Operand {
    Operand {
        ty: I32,
        kind: OperandKind::Constant(Constant::Scalar {
            bits: value,
            bytes: 4,
        }),
    }
}

/// `fn f(x: i32) -> Option<i32>`: switches on `x`, calls a direct item, calls a virtual method,
/// drops, and returns.
fn build_body(interns: &mut Interns) -> Body {
    let link = interns.intern_str("callee$h1");
    let member = interns.intern_str("map_get");
    let file = interns.intern_str("src/lib.rs");
    let location = interns.intern_source_fact(SourceFact {
        file,
        line: 3,
        col: 4,
    });
    let item = |n: u64| ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: n,
        },
        instantiation: n as u128,
    };
    Body {
        item: item(100),
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: Signature {
            arguments: vec![I32],
            result: OPTION_I32,
            abi: AbiSignature {
                parameters: 1,
                spread: None,
                caller_location: false,
                inline_never: false,
                ..AbiSignature::default()
            },
        },
        locals: vec![
            Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: OPTION_I32,
            },
            Local {
                id: LocalId(1),
                role: LocalRole::Argument,
                ty: I32,
            },
            Local {
                id: LocalId(2),
                role: LocalRole::Temporary,
                ty: I32,
            },
            Local {
                id: LocalId(3),
                role: LocalRole::Temporary,
                ty: DYN,
            },
            Local {
                id: LocalId(4),
                role: LocalRole::Temporary,
                ty: UNIT,
            },
        ],
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: vec![
                    Statement {
                        kind: StatementKind::StorageLive(LocalId(2)),
                    },
                    Statement {
                        kind: StatementKind::Assign {
                            destination: place(2, I32),
                            value: Rvalue::Binary {
                                op: BinaryOp::Add,
                                left: Operand {
                                    ty: I32,
                                    kind: OperandKind::Copy(place(1, I32)),
                                },
                                right: constant_i32(1),
                            },
                        },
                    },
                ],
                terminator: Terminator::Switch {
                    discriminant: Operand {
                        ty: I32,
                        kind: OperandKind::Copy(place(2, I32)),
                    },
                    targets: vec![(0, BlockId(1))],
                    otherwise: BlockId(2),
                },
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: vec![Statement {
                    kind: StatementKind::Assign {
                        destination: place(0, OPTION_I32),
                        value: Rvalue::Aggregate {
                            kind: AggregateKind::Adt {
                                definition: DefinitionId {
                                    crate_id: 1,
                                    local_hash: 10,
                                },
                                variant: 0,
                                active_field: None,
                            },
                            result: OPTION_I32,
                            operands: Vec::new(),
                        },
                    },
                }],
                terminator: Terminator::Return,
            },
            BasicBlock {
                id: BlockId(2),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: UNIT,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    callee: Callee::Direct {
                        item: item(200),
                        link_name: link,
                    },
                    semantic: CallSemantic::Ordinary,
                    arguments: vec![Operand {
                        ty: I32,
                        kind: OperandKind::Move(place(2, I32)),
                    }],
                    destination: place(0, OPTION_I32),
                    target: Some(BlockId(3)),
                    unwind: Unwind::Continue,
                    track_caller: Some(location),
                    host: None,
                }),
            },
            BasicBlock {
                id: BlockId(3),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: UNIT,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    callee: Callee::Virtual {
                        slot: 3,
                        receiver: DYN,
                    },
                    semantic: CallSemantic::Ordinary,
                    arguments: vec![Operand {
                        ty: DYN,
                        kind: OperandKind::Move(place(3, DYN)),
                    }],
                    destination: place(4, UNIT),
                    target: Some(BlockId(4)),
                    unwind: Unwind::Continue,
                    track_caller: None,
                    host: Some(HostPayload::Map { marker: member }),
                }),
            },
            BasicBlock {
                id: BlockId(4),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Drop {
                    place: place(0, OPTION_I32),
                    target: BlockId(1),
                    unwind: Unwind::Continue,
                    replace: false,
                    async_drop: None,
                    glue: DropGlue::None,
                },
            },
        ],
    }
}

fn build_origins(interns: &mut Interns, body: &Body) -> OriginTable {
    let path = interns.intern_str("demo::f");
    let name = interns.intern_str("x");
    let file = interns.intern_str("src/lib.rs");
    let fact = interns.intern_source_fact(SourceFact {
        file,
        line: 1,
        col: 0,
    });
    let origins = vec![
        Origin {
            kind: OriginKind::Statement,
            source: Some(fact),
            local_name: None,
            inline_stack: Vec::new(),
        },
        Origin {
            kind: OriginKind::Local,
            source: Some(fact),
            local_name: Some(name),
            inline_stack: vec![OriginId(0)],
        },
    ];
    OriginTable {
        origins,
        statements: vec![OriginId(0); body.statement_count()],
        terminators: vec![OriginId(0); body.blocks.len()],
        locals: vec![OriginId(1); body.locals.len()],
        item_path: path,
        root: Some(fact),
    }
}

#[test]
fn body_round_trips_and_verifies() {
    let mut interns = build_interns();
    let body = build_body(&mut interns);
    interns.types.verify().expect("type table verifies");
    body.verify(&interns.types, &interns)
        .expect("hand-built body verifies");

    let bytes = postcard::to_allocvec(&body).expect("serialize body");
    let decoded: Body = postcard::from_bytes(&bytes).expect("deserialize body");
    assert_eq!(decoded, body);
    decoded
        .verify(&interns.types, &interns)
        .expect("decoded body verifies");

    let table_bytes = postcard::to_allocvec(&interns).expect("serialize interns");
    let mut decoded_interns: Interns =
        postcard::from_bytes(&table_bytes).expect("deserialize interns");
    assert_eq!(decoded_interns.types, interns.types);
    assert_eq!(decoded_interns.strings, interns.strings);
    decoded_interns.rebuild_indices();
    assert_eq!(
        decoded_interns.intern_str("callee$h1"),
        interns.intern_str("callee$h1")
    );
    assert_eq!(decoded_interns.types.len(), 8);
    decoded_interns
        .types
        .verify()
        .expect("decoded table verifies");
}

#[test]
fn origins_are_outside_equality_and_check_lengths() {
    let mut interns = build_interns();
    let body = build_body(&mut interns);
    let origins = build_origins(&mut interns, &body);
    body.verify_origins(&origins).expect("origins match");

    let bytes = postcard::to_allocvec(&origins).expect("serialize origins");
    let decoded: OriginTable = postcard::from_bytes(&bytes).expect("deserialize origins");
    assert_eq!(decoded, origins);

    let mut short = origins;
    short.statements.pop();
    assert!(body.verify_origins(&short).is_err());

    // A second body with the same semantic content is equal regardless of its origins.
    let same = build_body(&mut interns);
    assert_eq!(same, body);
}

#[test]
fn verify_rejects_broken_bodies() {
    let mut interns = build_interns();
    let body = build_body(&mut interns);

    let mut swapped = body.clone();
    swapped.locals.swap(0, 1);
    assert!(swapped.verify(&interns.types, &interns).is_err());

    let mut missing_type = body.clone();
    missing_type.locals[2].ty = TypeId(999);
    let error = missing_type
        .verify(&interns.types, &interns)
        .expect_err("missing type");
    assert_eq!(
        error.message,
        format!("late-IR local references missing type {:032x}", 999)
    );

    let mut bad_block = body.clone();
    bad_block.blocks[0].terminator = Terminator::Goto(BlockId(42));
    assert_eq!(
        bad_block
            .verify(&interns.types, &interns)
            .expect_err("bad block")
            .message,
        "late-IR references missing block 42"
    );

    let mut bad_receiver = body.clone();
    if let Terminator::Call(call) = &mut bad_receiver.blocks[3].terminator {
        call.callee = Callee::Virtual {
            slot: 3,
            receiver: I32,
        };
    }
    assert_eq!(
        bad_receiver
            .verify(&interns.types, &interns)
            .expect_err("bad receiver")
            .message,
        "late-IR virtual call receiver is not a trait object"
    );

    let mut static_with_cfg = body.clone();
    static_with_cfg.kind = BodyKind::Static {
        init: ConstTree::Unit,
        mutable: false,
    };
    assert_eq!(
        static_with_cfg
            .verify(&interns.types, &interns)
            .expect_err("static with CFG")
            .message,
        "late-IR static contains a function CFG"
    );

    let mut bad_string = body;
    if let Terminator::Call(call) = &mut bad_string.blocks[2].terminator {
        call.callee = Callee::Runtime {
            member: super::interns::StrId(4242),
            sysroot: true,
        };
    }
    assert!(bad_string.verify(&interns.types, &interns).is_err());
}

#[test]
fn static_and_vtable_bodies_verify() {
    let mut interns = build_interns();
    let drop = interns.intern_str("drop$h");
    let method = interns.intern_str("method$h");
    let item = ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: 7,
        },
        instantiation: 7,
    };
    let signature = Signature {
        arguments: Vec::new(),
        result: I32,
        abi: AbiSignature::default(),
    };
    let static_body = Body {
        item,
        kind: BodyKind::Static {
            init: ConstTree::Scalar { bits: 5, bytes: 4 },
            mutable: true,
        },
        semantic: BodySemantic::Ordinary,
        signature: signature.clone(),
        locals: Vec::new(),
        blocks: Vec::new(),
    };
    static_body
        .verify(&interns.types, &interns)
        .expect("static verifies");

    let vtable = Body {
        item,
        kind: BodyKind::Vtable {
            concrete: I32,
            trait_ref: None,
            entries: vec![
                VtableEntry::Drop(Some(DirectItemRef {
                    item,
                    link_name: drop,
                })),
                VtableEntry::Size(4),
                VtableEntry::Align(4),
                VtableEntry::Method(DirectItemRef {
                    item,
                    link_name: method,
                }),
                VtableEntry::Vacant,
            ],
        },
        semantic: BodySemantic::Ordinary,
        signature,
        locals: Vec::new(),
        blocks: Vec::new(),
    };
    vtable
        .verify(&interns.types, &interns)
        .expect("vtable verifies");
    let bytes = postcard::to_allocvec(&vtable).expect("serialize vtable");
    let decoded: Body = postcard::from_bytes(&bytes).expect("deserialize vtable");
    assert_eq!(decoded, vtable);
}

#[test]
fn type_table_helpers_answer_like_v1() {
    let interns = build_interns();
    let types = &interns.types;
    assert!(types.is_indirect(OPTION_I32));
    assert!(!types.is_indirect(I32));
    assert!(!types.is_indirect(REF_I32));
    assert!(types.needs_clone(OPTION_I32));
    assert!(!types.needs_clone(REF_I32));
    assert!(types.is_zst(UNIT));
    assert_eq!(types.peeled(NONZERO_I32), I32);
    assert_eq!(types.int_info(NONZERO_I32), Some((true, 32)));
    assert_eq!(types.int_repr(I32), IntRepr::Number);
    assert_eq!(types.int_repr(OPTION_I32), IntRepr::Number);
    assert_eq!(
        types.enum_repr(OPTION_I32),
        Some(EnumRepr::Tag { fieldless: false })
    );
    assert_eq!(types.enum_repr(CORO), Some(EnumRepr::State));
    assert!(types.byte_element(U8));
    assert_eq!(types.pointer_form(REF_I32), Some(PointerForm::Slot));
    assert_eq!(types.variants(OPTION_I32).len(), 2);
    assert_eq!(types.variant_with_discriminant(OPTION_I32, 1), Some(1));
    assert_eq!(types.variant_with_discriminant(OPTION_I32, 9), None);

    // Field keys: an enum reads its downcast variant, a transparent wrapper is its field, a
    // coroutine reads upvars by index and saved locals by `$sN`.
    let some_payload = types.field(OPTION_I32, Some(1), 0).expect("Some._0");
    assert_eq!(some_payload.ty, I32);
    assert!(matches!(some_payload.key, FieldKey::Name(_)));
    assert_eq!(
        types.field(NONZERO_I32, None, 0).map(|field| field.key),
        Some(FieldKey::Transparent)
    );
    assert_eq!(
        types.field(UNIT, None, 0).map(|field| field.key),
        Some(FieldKey::Transparent)
    );
    assert_eq!(
        types.field(CORO, None, 0).map(|field| field.key),
        Some(FieldKey::Index(0))
    );
    let saved = types.field(CORO, Some(1), 1).expect("state 1 field 1");
    assert_eq!(saved.ty, U8);
    assert_eq!(
        saved.key,
        FieldKey::Name(interns.string_index_for_test("$s1"))
    );

    // Host parameter classes are computed from the encoded type.
    assert_eq!(HostParameterClass::of(I32, types), HostParameterClass::Sink);
    assert_eq!(
        HostParameterClass::of(REF_I32, types),
        HostParameterClass::Sink
    );
    assert_eq!(
        HostParameterClass::of(OPTION_I32, types),
        HostParameterClass::Pin
    );
    assert_eq!(
        HostParameterClass::of(UNIT, types),
        HostParameterClass::Sink
    );
}

#[test]
fn type_table_is_canonical_regardless_of_insertion_order() {
    let mut forward = TypeTable::new();
    let mut backward = TypeTable::new();
    let descriptors: Vec<_> = (1..=5).map(|id| scalar(id, TypeKind::Bool, 1)).collect();
    for descriptor in &descriptors {
        forward.insert(descriptor.clone());
    }
    for descriptor in descriptors.iter().rev() {
        backward.insert(descriptor.clone());
    }
    assert_eq!(forward, backward);
    assert!(!forward.insert(descriptors[0].clone()));
    forward.verify().expect("verifies");
    let bytes = postcard::to_allocvec(&forward).expect("serialize");
    let decoded: TypeTable = postcard::from_bytes(&bytes).expect("deserialize");
    assert_eq!(decoded, forward);
    assert_eq!(decoded.get(TypeId(3)).map(|d| d.id), Some(TypeId(3)));
}

#[test]
fn interns_dedup_and_keep_ids_stable() {
    let mut interns = Interns::new();
    let a = interns.intern_str("a");
    let b = interns.intern_str("b");
    assert_ne!(a, b);
    assert_eq!(interns.intern_str("a"), a);
    let bytes = interns.intern_bytes(b"xyz");
    assert_eq!(interns.intern_bytes(b"xyz"), bytes);
    assert_eq!(interns.bytes(bytes), Some(&b"xyz"[..]));
    let sig = SigDescriptor {
        inputs: vec![TypeId(1)],
        output: TypeId(2),
        abi: SigAbi::Rust,
        spread_arity: None,
        track_caller: false,
        unwind: true,
    };
    let id = interns.intern_sig(sig.clone());
    assert_eq!(interns.intern_sig(sig.clone()), id);
    assert_eq!(interns.sig(id), Some(&sig));
}

impl Interns {
    /// The id already interned for `value`, for tests that hold an immutable table.
    fn string_index_for_test(&self, value: &str) -> super::interns::StrId {
        let position = self
            .strings
            .iter()
            .position(|candidate| candidate == value)
            .expect("string was interned");
        super::interns::StrId(position as u32)
    }
}
