//! Interpreter and JIR-R tests: hand-built JIR-S bodies evaluated by the semantic interpreter
//! against pinned values, the same programs as JIR-R functions evaluated by the representation
//! interpreter with equal outcomes, and the JIR-R verifier's accept/reject cases.

use crate::jir::interns::{Interns, StrId};
use crate::jir::origin::OriginId;
use crate::jir::repr::*;
use crate::jir::semantic::*;
use crate::jir::types::*;

use super::*;

const UNIT: TypeId = TypeId(1);
const BOOL: TypeId = TypeId(2);
const I32: TypeId = TypeId(3);
const U8: TypeId = TypeId(4);
const USIZE: TypeId = TypeId(5);
const F32: TypeId = TypeId(6);
const OPTION: TypeId = TypeId(7);
const POINT: TypeId = TypeId(8);
const ARRAY: TypeId = TypeId(9);
const SLICE: TypeId = TypeId(10);
const REF_POINT: TypeId = TypeId(11);
const REF_I32: TypeId = TypeId(12);
const REF_SLICE: TypeId = TypeId(13);
const REF_ARRAY: TypeId = TypeId(14);
const OVERFLOW: TypeId = TypeId(15);
const ORDERING: TypeId = TypeId(16);
const I64: TypeId = TypeId(17);

fn descriptor(id: TypeId, kind: TypeKind, size: u64) -> TypeDescriptor {
    TypeDescriptor {
        id,
        name: None,
        kind,
        layout: LayoutFacts {
            size: Some(size),
            unsized_prefix: None,
            align: Some(size.clamp(1, 4)),
            sized: true,
            zst: size == 0,
            one_zst: size == 0,
            needs_drop: false,
            has_own_drop: false,
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int: None,
            is_indirect: size == 0,
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

fn int(id: TypeId, signed: bool, bits: u32) -> TypeDescriptor {
    let mut descriptor = descriptor(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
    descriptor.js.int = Some(IntFacts {
        signed,
        bits,
        repr: if bits > 32 {
            IntRepr::BigInt
        } else {
            IntRepr::Number
        },
    });
    descriptor.js.byte_element = bits == 8;
    descriptor
}

fn variant(name: StrId, fields: Vec<(FieldKey, TypeId)>, discr: i128) -> VariantDescriptor {
    VariantDescriptor {
        name,
        fields: fields
            .into_iter()
            .map(|(key, ty)| FieldDescriptor { key, ty, offset: 0 })
            .collect(),
        logical_discr: discr,
        physical_tag: None,
        result_ok: false,
        saved_local_keys: Vec::new(),
    }
}

fn reference(id: TypeId, pointee: TypeId, form: PointerForm) -> TypeDescriptor {
    let mut descriptor = descriptor(
        id,
        TypeKind::Reference {
            pointee,
            mutable: true,
        },
        4,
    );
    descriptor.js.pointer_form = Some(form);
    descriptor
}

/// `()`, `bool`, `i32`, `u8`, `usize`, `f32`, `Option<i32>`, `Point { x, y }`, `[i32; 4]`,
/// `[i32]`, `&mut Point`, `&mut i32`, `&[i32]`, `&[i32; 4]`, `(i32, bool)`, `Ordering`, `i64`.
fn world() -> Interns {
    let mut interns = Interns::new();
    let def = |n: u64| DefinitionId {
        crate_id: 1,
        local_hash: n,
    };
    let none = interns.intern_str("None");
    let some = interns.intern_str("Some");
    let field_0 = interns.intern_str("_0");
    let x = interns.intern_str("x");
    let y = interns.intern_str("y");
    let point = interns.intern_str("Point");
    let less = interns.intern_str("Less");
    let equal = interns.intern_str("Equal");
    let greater = interns.intern_str("Greater");

    interns.intern_type(descriptor(UNIT, TypeKind::Tuple(Vec::new()), 0));
    interns.intern_type(descriptor(BOOL, TypeKind::Bool, 1));
    interns.intern_type(int(I32, true, 32));
    interns.intern_type(int(U8, false, 8));
    interns.intern_type(int(USIZE, false, 32));
    interns.intern_type(int(I64, true, 64));
    interns.intern_type(descriptor(F32, TypeKind::Float { bits: 32 }, 4));

    let mut option = descriptor(
        OPTION,
        TypeKind::Adt {
            def: def(10),
            args: vec![I32],
            form: NominalForm::Enum,
            variants: vec![
                variant(none, Vec::new(), 0),
                variant(some, vec![(FieldKey::Name(field_0), I32)], 1),
            ],
            niche: None,
        },
        8,
    );
    option.js.is_indirect = true;
    option.js.needs_clone = true;
    option.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(option);

    let mut point_ty = descriptor(
        POINT,
        TypeKind::Adt {
            def: def(11),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                point,
                vec![(FieldKey::Name(x), I32), (FieldKey::Name(y), I32)],
                0,
            )],
            niche: None,
        },
        8,
    );
    point_ty.js.is_indirect = true;
    point_ty.js.needs_clone = true;
    point_ty.js.fields = vec![
        FieldDescriptor {
            key: FieldKey::Name(x),
            ty: I32,
            offset: 0,
        },
        FieldDescriptor {
            key: FieldKey::Name(y),
            ty: I32,
            offset: 4,
        },
    ];
    interns.intern_type(point_ty);

    let mut array = descriptor(
        ARRAY,
        TypeKind::Array {
            element: I32,
            length: 4,
        },
        16,
    );
    array.js.is_indirect = true;
    array.js.needs_clone = true;
    array.js.fields = vec![FieldDescriptor {
        key: FieldKey::Index(0),
        ty: I32,
        offset: 0,
    }];
    interns.intern_type(array);

    let mut slice = descriptor(SLICE, TypeKind::Slice(I32), 0);
    slice.layout.sized = false;
    slice.layout.size = None;
    slice.layout.zst = false;
    slice.js.is_indirect = true;
    interns.intern_type(slice);

    interns.intern_type(reference(REF_POINT, POINT, PointerForm::Thin));
    interns.intern_type(reference(REF_I32, I32, PointerForm::Slot));
    interns.intern_type(reference(REF_SLICE, SLICE, PointerForm::FatSlice));
    interns.intern_type(reference(REF_ARRAY, ARRAY, PointerForm::Thin));

    let mut overflow = descriptor(OVERFLOW, TypeKind::Tuple(vec![I32, BOOL]), 8);
    overflow.js.is_indirect = true;
    overflow.js.needs_clone = true;
    overflow.js.fields = vec![
        FieldDescriptor {
            key: FieldKey::Index(0),
            ty: I32,
            offset: 0,
        },
        FieldDescriptor {
            key: FieldKey::Index(1),
            ty: BOOL,
            offset: 4,
        },
    ];
    interns.intern_type(overflow);

    let mut ordering = descriptor(
        ORDERING,
        TypeKind::Adt {
            def: def(12),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(less, Vec::new(), -1),
                variant(equal, Vec::new(), 0),
                variant(greater, Vec::new(), 1),
            ],
            niche: None,
        },
        1,
    );
    ordering.js.enum_repr = Some(EnumRepr::Number {
        bits: 8,
        signed: true,
    });
    ordering.js.is_primitive_enum = true;
    interns.intern_type(ordering);
    interns
}

fn item(n: u64) -> ItemId {
    ItemId {
        definition: DefinitionId {
            crate_id: 1,
            local_hash: n,
        },
        instantiation: n as u128,
    }
}

fn signature(arguments: Vec<TypeId>, result: TypeId) -> Signature {
    Signature {
        abi: AbiSignature {
            parameters: arguments.len() as u16,
            spread: None,
            caller_location: false,
            inline_never: false,
            ..AbiSignature::default()
        },
        arguments,
        result,
    }
}

// JIR-S builders.

fn local(id: u32, role: LocalRole, ty: TypeId) -> Local {
    Local {
        id: LocalId(id),
        role,
        ty,
    }
}

fn place(local: u32, projection: Vec<Projection>, ty: TypeId) -> Place {
    Place {
        local: LocalId(local),
        projection,
        ty,
    }
}

fn copy(local: u32, ty: TypeId) -> Operand {
    Operand {
        ty,
        kind: OperandKind::Copy(place(local, Vec::new(), ty)),
    }
}

fn copy_place(place: Place) -> Operand {
    Operand {
        ty: place.ty,
        kind: OperandKind::Copy(place),
    }
}

fn scalar_operand(bits: u128, bytes: u8, ty: TypeId) -> Operand {
    Operand {
        ty,
        kind: OperandKind::Constant(Constant::Scalar { bits, bytes }),
    }
}

fn i32_const(value: i32) -> Operand {
    scalar_operand(value as u32 as u128, 4, I32)
}

fn assign(destination: Place, value: Rvalue) -> Statement {
    Statement {
        kind: StatementKind::Assign { destination, value },
    }
}

fn block(statements: Vec<Statement>, terminator: Terminator) -> BasicBlock {
    BasicBlock {
        id: BlockId(0),
        cleanup: false,
        statements,
        terminator,
    }
}

fn body(signature_of: Signature, locals: Vec<Local>, mut blocks: Vec<BasicBlock>) -> Body {
    for (index, block) in blocks.iter_mut().enumerate() {
        block.id = BlockId(index as u32);
    }
    Body {
        item: item(1),
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: signature_of,
        locals,
        blocks,
    }
}

fn i32_value(value: i32) -> Value {
    Value::int(value as u32 as u128, 32, true)
}

fn no_calls(interns: &Interns, body_: &Body, args: Vec<Value>) -> Outcome {
    let hook = |callee: &DirectItemRef, _: Vec<Value>| -> Result<Value, Trap> {
        Err(Trap::Hook(format!(
            "unexpected call to {}",
            callee.link_name.0
        )))
    };
    let hooks = Hooks::new(&hook);
    run_body(body_, &interns.types, interns, args, &hooks)
}

fn no_calls_func(interns: &Interns, func: &Func, args: Vec<Value>) -> Outcome {
    let hook = |callee: &DirectItemRef, _: Vec<Value>| -> Result<Value, Trap> {
        Err(Trap::Hook(format!(
            "unexpected call to {}",
            callee.link_name.0
        )))
    };
    let hooks = Hooks::new(&hook);
    run_func(func, &interns.types, interns, args, &hooks)
}

/// `fn arith(a: i32, b: i32) -> i32 { let (s, o) = a.overflowing_add(b); assert!(!o); s * 3 }`
/// (wrapping multiply), plus a `u8` shift and an `i64` mask.
fn arith_body() -> Body {
    // _0 ret, _1 a, _2 b, _3 (i32,bool), _4 bool, _5 i32
    let locals = vec![
        local(0, LocalRole::Return, I32),
        local(1, LocalRole::Argument, I32),
        local(2, LocalRole::Argument, I32),
        local(3, LocalRole::Temporary, OVERFLOW),
        local(4, LocalRole::Temporary, BOOL),
        local(5, LocalRole::Temporary, I32),
    ];
    let field = |base: u32, index: u32, ty: TypeId| {
        place(base, vec![Projection::Field { field: index, ty }], ty)
    };
    let entry = block(
        vec![
            assign(
                place(3, Vec::new(), OVERFLOW),
                Rvalue::Binary {
                    op: BinaryOp::AddWithOverflow,
                    left: copy(1, I32),
                    right: copy(2, I32),
                },
            ),
            assign(
                place(4, Vec::new(), BOOL),
                Rvalue::Use(copy_place(field(3, 1, BOOL))),
            ),
        ],
        Terminator::Assert {
            condition: copy(4, BOOL),
            expected: false,
            message: AssertMessage::Overflow {
                op: BinaryOp::Add,
                left: copy(1, I32),
                right: copy(2, I32),
            },
            target: BlockId(1),
            unwind: Unwind::Continue,
            track_caller: None,
        },
    );
    let rest = block(
        vec![
            assign(
                place(5, Vec::new(), I32),
                Rvalue::Use(copy_place(field(3, 0, I32))),
            ),
            assign(
                place(0, Vec::new(), I32),
                Rvalue::Binary {
                    op: BinaryOp::Mul,
                    left: copy(5, I32),
                    right: i32_const(3),
                },
            ),
        ],
        Terminator::Return,
    );
    body(signature(vec![I32, I32], I32), locals, vec![entry, rest])
}

#[test]
fn semantic_arith_wraps_and_asserts() {
    let interns = world();
    let arith = arith_body();
    arith.verify(&interns.types, &interns).unwrap();
    let outcome = no_calls(&interns, &arith, vec![i32_value(2), i32_value(5)]);
    assert_eq!(outcome.ret.unwrap(), i32_value(21));
    assert!(outcome.trace.is_empty());
    // (i32::MAX / 2 + 1) * 3 wraps.
    let outcome = no_calls(&interns, &arith, vec![i32_value(1 << 30), i32_value(0)]);
    assert_eq!(
        outcome.ret.unwrap(),
        i32_value((1_i32 << 30).wrapping_mul(3))
    );
    // Overflowing add trips the assert.
    let trap = no_calls(&interns, &arith, vec![i32_value(i32::MAX), i32_value(1)])
        .ret
        .unwrap_err();
    assert_eq!(trap, Trap::Assert("arithmetic overflow"));
}

#[test]
fn integer_operators_have_rust_semantics() {
    let interns = world();
    let table = &interns.types;
    let u8v = |v: u8| Value::int(u128::from(v), 8, false);
    let i64v = |v: i64| Value::int(v as u64 as u128, 64, true);
    // u8 wrapping add and shift by a masked amount.
    let sum = eval::binary(BinaryOp::Add, &u8v(200), &u8v(100), U8, U8, table).unwrap();
    assert_eq!(sum, u8v(44));
    let shifted = eval::binary(BinaryOp::Shl, &u8v(3), &u8v(9), U8, U8, table).unwrap();
    assert_eq!(shifted, u8v(6));
    // i64 arithmetic stays exact and wraps at 64 bits.
    let product = eval::binary(BinaryOp::Mul, &i64v(i64::MAX), &i64v(2), I64, I64, table).unwrap();
    assert_eq!(product, i64v(-2));
    let overflow = eval::binary(
        BinaryOp::MulWithOverflow,
        &i64v(1 << 62),
        &i64v(4),
        I64,
        OVERFLOW,
        table,
    )
    .unwrap();
    assert_eq!(
        overflow,
        Value::Aggregate {
            ty: OVERFLOW,
            variant: 0,
            fields: vec![i64v(0), Value::Bool(true)]
        }
    );
    // Signed division rounds toward zero, remainder keeps the dividend's sign.
    let quotient = eval::binary(
        BinaryOp::Div,
        &i32_value(-7),
        &i32_value(2),
        I32,
        I32,
        table,
    )
    .unwrap();
    assert_eq!(quotient, i32_value(-3));
    let remainder = eval::binary(
        BinaryOp::Rem,
        &i32_value(-7),
        &i32_value(2),
        I32,
        I32,
        table,
    )
    .unwrap();
    assert_eq!(remainder, i32_value(-1));
    assert_eq!(
        eval::binary(BinaryOp::Div, &i32_value(1), &i32_value(0), I32, I32, table),
        Err(Trap::Ub("division by zero"))
    );
    // Cmp yields the Ordering variant.
    let less = eval::binary(
        BinaryOp::Cmp,
        &i32_value(1),
        &i32_value(2),
        I32,
        ORDERING,
        table,
    )
    .unwrap();
    assert_eq!(
        less,
        Value::Aggregate {
            ty: ORDERING,
            variant: 0,
            fields: Vec::new()
        }
    );
    // Unsigned shift right is logical.
    let shr = eval::binary(BinaryOp::Shr, &u8v(0x80), &u8v(7), U8, U8, table).unwrap();
    assert_eq!(shr, u8v(1));
    // f32 arithmetic rounds to single precision after every op.
    let heap = Heap::new();
    let third = eval::binary(
        BinaryOp::Div,
        &Value::float(1.0, 32),
        &Value::float(3.0, 32),
        F32,
        F32,
        table,
    )
    .unwrap();
    assert_eq!(third, Value::float(f64::from(1.0_f32 / 3.0_f32), 32));
    // Casts: saturating float to int, sign extension, truncation.
    assert_eq!(
        eval::cast(
            CastKind::FloatToInt,
            &Value::float(1e10, 64),
            I32,
            table,
            &heap
        )
        .unwrap(),
        i32_value(i32::MAX)
    );
    assert_eq!(
        eval::cast(
            CastKind::FloatToInt,
            &Value::float(f64::NAN, 64),
            I32,
            table,
            &heap
        )
        .unwrap(),
        i32_value(0)
    );
    assert_eq!(
        eval::cast(CastKind::IntToInt, &i32_value(-1), U8, table, &heap).unwrap(),
        u8v(255)
    );
    assert_eq!(
        eval::cast(CastKind::IntToInt, &i32_value(-1), I64, table, &heap).unwrap(),
        i64v(-1)
    );
    assert_eq!(
        eval::cast(CastKind::IntToInt, &u8v(255), I32, table, &heap).unwrap(),
        i32_value(255)
    );
    assert_eq!(
        eval::unary(UnaryOp::Neg, &i32_value(i32::MIN)).unwrap(),
        i32_value(i32::MIN)
    );
    assert_eq!(eval::unary(UnaryOp::Not, &u8v(0x0f)).unwrap(), u8v(0xf0));
}

/// `fn unwrap_or_neg(o: Option<i32>) -> i32 { match o { Some(v) => v, None => -1 } }`.
fn enum_body() -> Body {
    // _0 ret, _1 o, _2 discr (i32 in this world), _3 v
    let locals = vec![
        local(0, LocalRole::Return, I32),
        local(1, LocalRole::Argument, OPTION),
        local(2, LocalRole::Temporary, I32),
        local(3, LocalRole::Temporary, I32),
    ];
    let entry = block(
        vec![assign(
            place(2, Vec::new(), I32),
            Rvalue::Discriminant(place(1, Vec::new(), OPTION)),
        )],
        Terminator::Switch {
            discriminant: copy(2, I32),
            targets: vec![(1, BlockId(1))],
            otherwise: BlockId(2),
        },
    );
    let some = block(
        vec![
            assign(
                place(3, Vec::new(), I32),
                Rvalue::Use(copy_place(place(
                    1,
                    vec![
                        Projection::Downcast { variant: 1 },
                        Projection::Field { field: 0, ty: I32 },
                    ],
                    I32,
                ))),
            ),
            assign(place(0, Vec::new(), I32), Rvalue::Use(copy(3, I32))),
        ],
        Terminator::Return,
    );
    let none = block(
        vec![assign(
            place(0, Vec::new(), I32),
            Rvalue::Use(i32_const(-1)),
        )],
        Terminator::Return,
    );
    body(
        signature(vec![OPTION], I32),
        locals,
        vec![entry, some, none],
    )
}

fn some(value: i32) -> Value {
    Value::Aggregate {
        ty: OPTION,
        variant: 1,
        fields: vec![i32_value(value)],
    }
}

fn none() -> Value {
    Value::Aggregate {
        ty: OPTION,
        variant: 0,
        fields: Vec::new(),
    }
}

#[test]
fn semantic_enum_switch_and_downcast() {
    let interns = world();
    let enum_ = enum_body();
    enum_.verify(&interns.types, &interns).unwrap();
    assert_eq!(
        no_calls(&interns, &enum_, vec![some(7)]).ret.unwrap(),
        i32_value(7)
    );
    assert_eq!(
        no_calls(&interns, &enum_, vec![none()]).ret.unwrap(),
        i32_value(-1)
    );
}

/// `fn through_ref() -> i32 { let mut p = Point { x: 1, y: 2 }; let r = &mut p; r.y = 5; p.x + p.y
/// }`.
fn struct_ref_body() -> Body {
    // _0 ret, _1 p, _2 r
    let locals = vec![
        local(0, LocalRole::Return, I32),
        local(1, LocalRole::Temporary, POINT),
        local(2, LocalRole::Temporary, REF_POINT),
    ];
    let point_def = DefinitionId {
        crate_id: 1,
        local_hash: 11,
    };
    let entry = block(
        vec![
            assign(
                place(1, Vec::new(), POINT),
                Rvalue::Aggregate {
                    kind: AggregateKind::Adt {
                        definition: point_def,
                        variant: 0,
                        active_field: None,
                    },
                    result: POINT,
                    operands: vec![i32_const(1), i32_const(2)],
                },
            ),
            assign(
                place(2, Vec::new(), REF_POINT),
                Rvalue::Reference {
                    kind: BorrowKind::Mutable,
                    place: place(1, Vec::new(), POINT),
                },
            ),
            assign(
                place(
                    2,
                    vec![Projection::Deref, Projection::Field { field: 1, ty: I32 }],
                    I32,
                ),
                Rvalue::Use(i32_const(5)),
            ),
            assign(
                place(0, Vec::new(), I32),
                Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: copy_place(place(1, vec![Projection::Field { field: 0, ty: I32 }], I32)),
                    right: copy_place(place(1, vec![Projection::Field { field: 1, ty: I32 }], I32)),
                },
            ),
        ],
        Terminator::Return,
    );
    body(signature(Vec::new(), I32), locals, vec![entry])
}

#[test]
fn semantic_struct_field_write_through_reference() {
    let interns = world();
    let struct_ref = struct_ref_body();
    struct_ref.verify(&interns.types, &interns).unwrap();
    assert_eq!(
        no_calls(&interns, &struct_ref, Vec::new()).ret.unwrap(),
        i32_value(6)
    );
}

/// `fn index(i: usize) -> i32 { let a = [10, 20, 30, 40]; let s: &[i32] = &a; s[i] }` with the
/// bounds assert MIR emits.
fn slice_body() -> Body {
    // _0 ret, _1 i, _2 a, _3 &[i32;4], _4 &[i32], _5 len, _6 in-bounds
    let locals = vec![
        local(0, LocalRole::Return, I32),
        local(1, LocalRole::Argument, USIZE),
        local(2, LocalRole::Temporary, ARRAY),
        local(3, LocalRole::Temporary, REF_ARRAY),
        local(4, LocalRole::Temporary, REF_SLICE),
        local(5, LocalRole::Temporary, USIZE),
        local(6, LocalRole::Temporary, BOOL),
    ];
    let entry = block(
        vec![
            assign(
                place(2, Vec::new(), ARRAY),
                Rvalue::Aggregate {
                    kind: AggregateKind::Array(I32),
                    result: ARRAY,
                    operands: vec![i32_const(10), i32_const(20), i32_const(30), i32_const(40)],
                },
            ),
            assign(
                place(3, Vec::new(), REF_ARRAY),
                Rvalue::Reference {
                    kind: BorrowKind::Shared,
                    place: place(2, Vec::new(), ARRAY),
                },
            ),
            assign(
                place(4, Vec::new(), REF_SLICE),
                Rvalue::Cast {
                    kind: CastKind::PointerCoercion,
                    value: copy(3, REF_ARRAY),
                    result: REF_SLICE,
                },
            ),
            assign(
                place(5, Vec::new(), USIZE),
                Rvalue::Unary {
                    op: UnaryOp::PtrMetadata,
                    value: copy(4, REF_SLICE),
                },
            ),
            assign(
                place(6, Vec::new(), BOOL),
                Rvalue::Binary {
                    op: BinaryOp::Lt,
                    left: copy(1, USIZE),
                    right: copy(5, USIZE),
                },
            ),
        ],
        Terminator::Assert {
            condition: copy(6, BOOL),
            expected: true,
            message: AssertMessage::BoundsCheck {
                length: copy(5, USIZE),
                index: copy(1, USIZE),
            },
            target: BlockId(1),
            unwind: Unwind::Continue,
            track_caller: None,
        },
    );
    let read = block(
        vec![assign(
            place(0, Vec::new(), I32),
            Rvalue::Use(copy_place(place(
                4,
                vec![Projection::Deref, Projection::Index(LocalId(1))],
                I32,
            ))),
        )],
        Terminator::Return,
    );
    body(signature(vec![USIZE], I32), locals, vec![entry, read])
}

#[test]
fn semantic_slice_index_with_bounds_check() {
    let interns = world();
    let slice = slice_body();
    slice.verify(&interns.types, &interns).unwrap();
    assert_eq!(
        no_calls(&interns, &slice, vec![Value::usize(2)])
            .ret
            .unwrap(),
        i32_value(30)
    );
    assert_eq!(
        no_calls(&interns, &slice, vec![Value::usize(4)])
            .ret
            .unwrap_err(),
        Trap::Assert("bounds check")
    );
}

/// `fn cell() -> i32 { let mut x = 1; let r = &mut x; *r = *r + 41; x }`.
fn cell_body() -> Body {
    // _0 ret, _1 x, _2 r
    let locals = vec![
        local(0, LocalRole::Return, I32),
        local(1, LocalRole::Temporary, I32),
        local(2, LocalRole::Temporary, REF_I32),
    ];
    let deref = place(2, vec![Projection::Deref], I32);
    let entry = block(
        vec![
            assign(place(1, Vec::new(), I32), Rvalue::Use(i32_const(1))),
            assign(
                place(2, Vec::new(), REF_I32),
                Rvalue::Reference {
                    kind: BorrowKind::Mutable,
                    place: place(1, Vec::new(), I32),
                },
            ),
            assign(
                deref.clone(),
                Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: copy_place(deref),
                    right: i32_const(41),
                },
            ),
            assign(place(0, Vec::new(), I32), Rvalue::Use(copy(1, I32))),
        ],
        Terminator::Return,
    );
    body(signature(Vec::new(), I32), locals, vec![entry])
}

#[test]
fn semantic_cell_write_through_reference() {
    let interns = world();
    let cell = cell_body();
    cell.verify(&interns.types, &interns).unwrap();
    assert_eq!(
        no_calls(&interns, &cell, Vec::new()).ret.unwrap(),
        i32_value(42)
    );
}

#[test]
fn semantic_traces_calls_and_drops() {
    let interns = world();
    let mut interns_mut = interns;
    let link = interns_mut.intern_str("callee$h1");
    let interns = interns_mut;
    // _0 ret, _1 arg, _2 point (dropped)
    let locals = vec![
        local(0, LocalRole::Return, I32),
        local(1, LocalRole::Argument, I32),
        local(2, LocalRole::Temporary, POINT),
    ];
    let entry = block(
        vec![assign(
            place(2, Vec::new(), POINT),
            Rvalue::Aggregate {
                kind: AggregateKind::Adt {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 11,
                    },
                    variant: 0,
                    active_field: None,
                },
                result: POINT,
                operands: vec![copy(1, I32), i32_const(0)],
            },
        )],
        Terminator::Call(Call {
            function: Operand {
                ty: UNIT,
                kind: OperandKind::Constant(Constant::ZeroSized),
            },
            callee: Callee::Direct {
                item: item(9),
                link_name: link,
            },
            semantic: CallSemantic::Ordinary,
            arguments: vec![copy(1, I32)],
            destination: place(0, Vec::new(), I32),
            target: Some(BlockId(1)),
            unwind: Unwind::Continue,
            track_caller: None,
            host: None,
        }),
    );
    let drop = block(
        Vec::new(),
        Terminator::Drop {
            place: place(2, Vec::new(), POINT),
            target: BlockId(2),
            unwind: Unwind::Continue,
            replace: false,
            async_drop: None,
            glue: DropGlue::Direct {
                item: item(8),
                link_name: link,
            },
        },
    );
    let ret = block(Vec::new(), Terminator::Return);
    let traced = body(signature(vec![I32], I32), locals, vec![entry, drop, ret]);
    let hook = |callee: &DirectItemRef, args: Vec<Value>| -> Result<Value, Trap> {
        assert_eq!(callee.link_name, link);
        let arg = args[0].as_int().unwrap();
        Ok(i32_value(arg as i32 * 2))
    };
    let hooks = Hooks::new(&hook);
    let outcome = run_body(
        &traced,
        &interns.types,
        &interns,
        vec![i32_value(21)],
        &hooks,
    );
    assert_eq!(outcome.ret.unwrap(), i32_value(42));
    assert_eq!(
        outcome.trace,
        vec![
            Event::Call {
                callee: DirectItemRef {
                    item: item(9),
                    link_name: link
                },
                args: vec![i32_value(21)],
                result: Ok(i32_value(42)),
            },
            Event::Drop {
                ty: POINT,
                value: Value::Aggregate {
                    ty: POINT,
                    variant: 0,
                    fields: vec![i32_value(21), i32_value(0)]
                },
            },
        ]
    );
}

#[test]
fn failed_hooks_remain_in_the_call_trace() {
    let mut interns = world();
    let name = interns.intern_str("throws");
    let callee = DirectItemRef {
        item: item(9),
        link_name: name,
    };
    let source = body(
        signature(vec![I32], I32),
        vec![
            local(0, LocalRole::Return, I32),
            local(1, LocalRole::Argument, I32),
        ],
        vec![
            block(
                Vec::new(),
                Terminator::Call(Call {
                    callee: Callee::Direct {
                        item: callee.item,
                        link_name: name,
                    },
                    function: Operand {
                        ty: UNIT,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    arguments: vec![copy(1, I32)],
                    destination: place(0, vec![], I32),
                    target: Some(BlockId(1)),
                    unwind: Unwind::Unreachable,
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                }),
            ),
            block(Vec::new(), Terminator::Return),
        ],
    );
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let arg = b.param(I32, ORIGIN);
    let result = b.op(
        FuncBuilder::ENTRY,
        OpKind::Call {
            callee: Callee::Direct {
                item: callee.item,
                link_name: name,
            },
            function: None,
            args: vec![arg],
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        ORIGIN,
    );
    b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(result)));
    let func = b.finish();
    source.verify(&interns.types, &interns).unwrap();
    func.verify(&interns.types).unwrap();
    let hook = |_: &DirectItemRef, _: Vec<Value>| Err(Trap::Unwind);
    let hooks = Hooks::new(&hook);
    let semantic = run_body(
        &source,
        &interns.types,
        &interns,
        vec![i32_value(21)],
        &hooks,
    );
    let repr = run_func(&func, &interns.types, &interns, vec![i32_value(21)], &hooks);
    assert_eq!(semantic, repr);
    assert_eq!(semantic.ret, Err(Trap::Unwind));
    assert_eq!(
        semantic.trace,
        vec![Event::Call {
            callee,
            args: vec![i32_value(21)],
            result: Err(Trap::Unwind),
        }]
    );
}

#[test]
fn memory_constants_reconstruct_transparent_wrappers() {
    let mut interns = world();
    let wrapper = TypeId(18);
    let name = interns.intern_str("Wrapper");
    let mut ty = descriptor(
        wrapper,
        TypeKind::Adt {
            def: item(18).definition,
            args: vec![OPTION],
            form: NominalForm::Struct,
            variants: vec![variant(
                name,
                vec![(FieldKey::Index(0), UNIT), (FieldKey::Index(1), OPTION)],
                0,
            )],
            niche: None,
        },
        8,
    );
    ty.js.transparent = Some(TransparentField {
        field: 1,
        ty: OPTION,
    });
    ty.js.peeled = OPTION;
    interns.intern_type(ty);
    for (variant, fields) in [
        (0, vec![]),
        (1, vec![ConstTree::Scalar { bits: 42, bytes: 4 }]),
    ] {
        let tree = ConstTree::Variant { variant, fields };
        let expected = Value::Aggregate {
            ty: wrapper,
            variant: 0,
            fields: vec![
                Value::Unit,
                Value::Aggregate {
                    ty: OPTION,
                    variant,
                    fields: if variant == 0 {
                        vec![]
                    } else {
                        vec![i32_value(42)]
                    },
                },
            ],
        };
        assert_eq!(
            eval::const_tree(&tree, wrapper, &interns.types, &interns),
            Ok(expected)
        );
    }
    assert!(matches!(
        eval::const_tree(
            &ConstTree::Variant {
                variant: 2,
                fields: vec![]
            },
            wrapper,
            &interns.types,
            &interns,
        ),
        Err(Trap::Malformed(_))
    ));
}

#[test]
fn heap_write_history_survives_same_values_and_restoration() {
    let interns = world();
    let mut heap = Heap::new();
    let original = Value::Aggregate {
        ty: POINT,
        variant: 0,
        fields: vec![i32_value(1), i32_value(2)],
    };
    let id = heap.alloc(original.clone());
    let root = Location::root(id);
    let field = root.child(Step::Field(0));
    let other = heap.alloc(i32_value(1));
    assert_eq!(heap.read(&root).unwrap(), original);
    {
        let slot = heap
            .slot(&field, Some(&[(POINT, None)]), &interns.types)
            .unwrap();
        assert_eq!(*slot, i32_value(1));
    }
    assert!(!heap.was_written(id));
    heap.write(&field, i32_value(1)).unwrap();
    assert!(heap.was_written(id));
    assert!(!heap.was_written(other));
    heap.write(&field, i32_value(42)).unwrap();
    heap.write(&field, i32_value(1)).unwrap();
    assert_eq!(heap.read(&root).unwrap(), original);
    assert!(heap.was_written(id));

    let uninitialized = heap.alloc(Value::Undef);
    let field = Location::root(uninitialized).child(Step::Field(0));
    heap.slot(&field, Some(&[(POINT, None)]), &interns.types)
        .unwrap();
    assert!(heap.was_written(uninitialized));
    assert_eq!(heap.read(&field).unwrap(), Value::Undef);

    let enum_id = heap.alloc(Value::Aggregate {
        ty: OPTION,
        variant: 0,
        fields: vec![],
    });
    let enum_location = Location::root(enum_id);
    {
        let mut slot = heap.slot(&enum_location, None, &interns.types).unwrap();
        eval::set_discriminant(&mut slot, 0, OPTION, &interns.types).unwrap();
    }
    assert!(heap.was_written(enum_id));

    let untouched = heap.alloc(original);
    let invalid = Location::root(untouched).child(Step::Field(9));
    assert!(heap.slot(&invalid, None, &interns.types).is_err());
    assert!(!heap.was_written(untouched));
}

// JIR-R.

const ORIGIN: OriginId = OriginId(0);

/// The JIR-R lowering of [`arith_body`].
fn arith_func() -> Func {
    let mut b = FuncBuilder::new(item(1), signature(vec![I32, I32], I32));
    let entry = FuncBuilder::ENTRY;
    let a = b.param(I32, ORIGIN);
    let bb = b.param(I32, ORIGIN);
    let sum = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::AddWithOverflow,
            l: a,
            r: bb,
            ty: I32,
        },
        OVERFLOW,
        ORIGIN,
    );
    let overflowed = b.op(
        entry,
        OpKind::Field {
            base: sum,
            variant: None,
            field: 1,
        },
        BOOL,
        ORIGIN,
    );
    b.effect(
        entry,
        OpKind::Assert {
            cond: overflowed,
            expected: false,
            msg: RAssertMessage::Overflow {
                op: BinaryOp::Add,
                left: a,
                right: bb,
            },
            track_caller: None,
        },
        ORIGIN,
    );
    let rest = b.block();
    b.terminate(entry, RTerm::Goto(rest, Vec::new()));
    let value = b.op(
        rest,
        OpKind::Field {
            base: sum,
            variant: None,
            field: 0,
        },
        I32,
        ORIGIN,
    );
    let three = b.op(
        rest,
        OpKind::Const(ConstTree::Scalar { bits: 3, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let product = b.op(
        rest,
        OpKind::Binary {
            op: BinaryOp::Mul,
            l: value,
            r: three,
            ty: I32,
        },
        I32,
        ORIGIN,
    );
    b.terminate(rest, RTerm::Return(Some(product)));
    b.finish()
}

/// The JIR-R lowering of [`enum_body`], with the result as a block argument.
fn enum_func() -> Func {
    let mut b = FuncBuilder::new(item(1), signature(vec![OPTION], I32));
    let entry = FuncBuilder::ENTRY;
    let o = b.param(OPTION, ORIGIN);
    let discr = b.op(entry, OpKind::Discriminant(o), I32, ORIGIN);
    let some = b.block();
    let none = b.block();
    let join = b.block();
    let result = b.block_param(join, I32, ORIGIN);
    b.terminate(
        entry,
        RTerm::Switch {
            scrut: discr,
            scrut_ty: I32,
            cases: vec![(1, some, Vec::new())],
            default: (none, Vec::new()),
        },
    );
    let v = b.op(
        some,
        OpKind::Field {
            base: o,
            variant: Some(1),
            field: 0,
        },
        I32,
        ORIGIN,
    );
    b.terminate(some, RTerm::Goto(join, vec![v]));
    let minus_one = b.op(
        none,
        OpKind::Const(ConstTree::Scalar {
            bits: u32::MAX as u128,
            bytes: 4,
        }),
        I32,
        ORIGIN,
    );
    b.terminate(none, RTerm::Goto(join, vec![minus_one]));
    b.terminate(join, RTerm::Return(Some(result)));
    b.finish()
}

/// The JIR-R lowering of [`cell_body`]: `x` is a boxed cell.
fn cell_func() -> Func {
    let mut b = FuncBuilder::new(item(1), signature(Vec::new(), I32));
    let entry = FuncBuilder::ENTRY;
    let x = b.cell(I32, true, ORIGIN);
    b.effect(entry, OpKind::CellAlloc(x), ORIGIN);
    let one = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        ORIGIN,
    );
    b.effect(entry, OpKind::CellStore { cell: x, v: one }, ORIGIN);
    let r = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Cell(x),
            path: Vec::new(),
            mutable: true,
        },
        REF_I32,
        ORIGIN,
    );
    let current = b.op(entry, OpKind::Deref(r), I32, ORIGIN);
    let forty_one = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 41, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let sum = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: current,
            r: forty_one,
            ty: I32,
        },
        I32,
        ORIGIN,
    );
    b.effect(entry, OpKind::StoreThrough { ptr: r, v: sum }, ORIGIN);
    let result = b.op(entry, OpKind::CellLoad(x), I32, ORIGIN);
    b.terminate(entry, RTerm::Return(Some(result)));
    b.finish()
}

/// The JIR-R lowering of [`struct_ref_body`] with the struct as an SSA value written through
/// `SetField` on a borrow.
fn struct_ref_func() -> Func {
    let mut b = FuncBuilder::new(item(1), signature(Vec::new(), I32));
    let entry = FuncBuilder::ENTRY;
    let one = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let two = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 2, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let p = b.op(
        entry,
        OpKind::Aggregate {
            ty: POINT,
            variant: 0,
            active_field: None,
            fields: vec![one, two],
        },
        POINT,
        ORIGIN,
    );
    let r = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(p),
            path: Vec::new(),
            mutable: true,
        },
        REF_POINT,
        ORIGIN,
    );
    let five = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let field_ref = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(r),
            path: vec![Proj::Deref, Proj::Field { field: 1, ty: I32 }],
            mutable: true,
        },
        REF_I32,
        ORIGIN,
    );
    b.effect(
        entry,
        OpKind::StoreThrough {
            ptr: field_ref,
            v: five,
        },
        ORIGIN,
    );
    let x = b.op(
        entry,
        OpKind::Field {
            base: p,
            variant: None,
            field: 0,
        },
        I32,
        ORIGIN,
    );
    let y = b.op(
        entry,
        OpKind::Field {
            base: p,
            variant: None,
            field: 1,
        },
        I32,
        ORIGIN,
    );
    let sum = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: x,
            r: y,
            ty: I32,
        },
        I32,
        ORIGIN,
    );
    b.terminate(entry, RTerm::Return(Some(sum)));
    b.finish()
}

#[test]
fn repr_and_semantic_interpreters_agree() {
    let interns = world();
    let cases: Vec<(Body, Func, Vec<Vec<Value>>)> = vec![
        (
            arith_body(),
            arith_func(),
            vec![
                vec![i32_value(2), i32_value(5)],
                vec![i32_value(1 << 30), i32_value(0)],
                vec![i32_value(i32::MAX), i32_value(1)],
            ],
        ),
        (enum_body(), enum_func(), vec![vec![some(7)], vec![none()]]),
        (cell_body(), cell_func(), vec![Vec::new()]),
        (struct_ref_body(), struct_ref_func(), vec![Vec::new()]),
    ];
    for (body_, func, arg_sets) in cases {
        body_.verify(&interns.types, &interns).unwrap();
        func.verify(&interns.types).unwrap();
        for args in arg_sets {
            let semantic = no_calls(&interns, &body_, args.clone());
            let repr = no_calls_func(&interns, &func, args);
            assert_eq!(semantic, repr);
        }
    }
}

#[test]
fn repr_slice_view_and_moves() {
    let interns = world();
    // fn f(i: usize) -> i32 { let a = [10,20,30,40]; let s: &[i32] = &a; s[i] } with a Move.
    let mut b = FuncBuilder::new(item(1), signature(vec![USIZE], I32));
    let entry = FuncBuilder::ENTRY;
    let i = b.param(USIZE, ORIGIN);
    let elements: Vec<ValueId> = [10, 20, 30, 40]
        .into_iter()
        .map(|n| {
            b.op(
                entry,
                OpKind::Const(ConstTree::Scalar { bits: n, bytes: 4 }),
                I32,
                ORIGIN,
            )
        })
        .collect();
    let array = b.op(
        entry,
        OpKind::Aggregate {
            ty: ARRAY,
            variant: 0,
            active_field: None,
            fields: elements,
        },
        ARRAY,
        ORIGIN,
    );
    let moved = b.op(entry, OpKind::Move(array), ARRAY, ORIGIN);
    let array_ref = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(moved),
            path: Vec::new(),
            mutable: false,
        },
        REF_ARRAY,
        ORIGIN,
    );
    let slice = b.op(
        entry,
        OpKind::Cast {
            kind: CastKind::PointerCoercion,
            v: array_ref,
            from: REF_ARRAY,
            to: REF_SLICE,
        },
        REF_SLICE,
        ORIGIN,
    );
    let len = b.op(
        entry,
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v: slice,
            ty: REF_SLICE,
        },
        USIZE,
        ORIGIN,
    );
    let in_bounds = b.op(
        entry,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: i,
            r: len,
            ty: USIZE,
        },
        BOOL,
        ORIGIN,
    );
    b.effect(
        entry,
        OpKind::Assert {
            cond: in_bounds,
            expected: true,
            msg: RAssertMessage::BoundsCheck {
                length: len,
                index: i,
            },
            track_caller: None,
        },
        ORIGIN,
    );
    let element_ref = b.op(
        entry,
        OpKind::BorrowView {
            root: Root::Value(slice),
            path: vec![Proj::Deref, Proj::Index(i)],
            mutable: false,
        },
        REF_I32,
        ORIGIN,
    );
    let element = b.op(entry, OpKind::Deref(element_ref), I32, ORIGIN);
    b.terminate(entry, RTerm::Return(Some(element)));
    let func = b.finish();
    func.verify(&interns.types).unwrap();
    let semantic = slice_body();
    for index in [0_u64, 2, 4] {
        assert_eq!(
            no_calls_func(&interns, &func, vec![Value::usize(index)]),
            no_calls(&interns, &semantic, vec![Value::usize(index)])
        );
    }
    // A use of the moved-from array is rejected.
    let mut broken = func.clone();
    broken.blocks[0].ops.push(Op {
        result: None,
        origin: ORIGIN,
        kind: OpKind::Drop {
            root: Root::Value(array),
            path: Vec::new(),
            glue: DropGlue::None,
        },
    });
    let error = broken.verify(&interns.types).unwrap_err();
    assert!(error.message.contains("after being moved"), "{error}");
}

#[test]
fn repr_loop_with_block_arguments() {
    let interns = world();
    // fn sum_to(n: i32) -> i32 { let mut acc = 0; let mut i = 0; while i < n { acc += i; i += 1 }
    // acc }
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let entry = FuncBuilder::ENTRY;
    let n = b.param(I32, ORIGIN);
    let zero = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let header = b.block();
    let acc = b.block_param(header, I32, ORIGIN);
    let i = b.block_param(header, I32, ORIGIN);
    let body_ = b.block();
    let exit = b.block();
    b.terminate(entry, RTerm::Goto(header, vec![zero, zero]));
    let cond = b.op(
        header,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: i,
            r: n,
            ty: I32,
        },
        BOOL,
        ORIGIN,
    );
    b.terminate(
        header,
        RTerm::Switch {
            scrut: cond,
            scrut_ty: BOOL,
            cases: vec![(0, exit, Vec::new())],
            default: (body_, Vec::new()),
        },
    );
    let acc2 = b.op(
        body_,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: acc,
            r: i,
            ty: I32,
        },
        I32,
        ORIGIN,
    );
    let one = b.op(
        body_,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let i2 = b.op(
        body_,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: one,
            ty: I32,
        },
        I32,
        ORIGIN,
    );
    b.terminate(body_, RTerm::Goto(header, vec![acc2, i2]));
    b.terminate(exit, RTerm::Return(Some(acc)));
    let func = b.finish();
    func.verify(&interns.types).unwrap();
    assert_eq!(
        no_calls_func(&interns, &func, vec![i32_value(5)])
            .ret
            .unwrap(),
        i32_value(10)
    );
    let doms = Dominators::of(&func);
    assert_eq!(doms.rpo, vec![entry, header, body_, exit]);
    assert!(doms.dominates(header, exit));
    assert!(!doms.dominates(body_, exit));

    // A value defined in the loop body does not dominate the exit.
    let mut broken = func.clone();
    broken.blocks[exit.0 as usize].term = RTerm::Return(Some(acc2));
    let error = broken.verify(&interns.types).unwrap_err();
    assert!(error.message.contains("not dominated"), "{error}");

    // The step budget catches an endless loop.
    let mut endless = func.clone();
    endless.blocks[body_.0 as usize].term = RTerm::Goto(header, vec![acc2, i]);
    let hook = |_: &DirectItemRef, _: Vec<Value>| -> Result<Value, Trap> {
        Err(Trap::Hook(String::new()))
    };
    let hooks = Hooks {
        call: &hook,
        drop: None,
        step_limit: 1_000,
    };
    assert_eq!(
        run_func(
            &endless,
            &interns.types,
            &interns,
            vec![i32_value(5)],
            &hooks
        )
        .ret
        .unwrap_err(),
        Trap::Steps
    );
}

#[test]
fn repr_verifier_rejects_broken_functions() {
    let interns = world();
    let table = &interns.types;
    let good = enum_func();
    good.verify(table).unwrap();

    let expect = |mutate: &dyn Fn(&mut Func), needle: &str| {
        let mut func = good.clone();
        mutate(&mut func);
        let error = func.verify(table).unwrap_err();
        assert!(error.message.contains(needle), "{needle:?} not in {error}");
    };

    expect(
        &|f| f.values[0].carrier = Carrier::NativeVec,
        "no committed plan allows",
    );
    // The same function verifies once a plan says it committed that carrier, which is the one
    // door a non-baseline carrier comes through.
    {
        let mut func = good.clone();
        func.values[0].carrier = Carrier::NativeVec;
        func.verify_carriers(table, CarrierPolicy::BASELINE.with(Carrier::NativeVec))
            .unwrap();
    }

    // A `Convert` names both ends so the emitter picks an adapter without walking back to the
    // definition, so both ends are checked against the values they claim to describe. `v1` is the
    // `Discriminant` the entry block computes, converted in place of the `-1` constant the
    // default arm produces; both are `i32`, which is what a carrier conversion keeps.
    let convert = |from, to, operand, result| {
        move |f: &mut Func| {
            f.values[1].carrier = operand;
            f.values[4].carrier = result;
            f.blocks[2].ops[0].kind = OpKind::Convert {
                v: ValueId(1),
                from,
                to,
            };
        }
    };
    let policy = CarrierPolicy::BASELINE.with(Carrier::HostString);
    let with_policy = |mutate: &dyn Fn(&mut Func), needle: &str| {
        let mut func = good.clone();
        mutate(&mut func);
        let error = func.verify_carriers(table, policy).unwrap_err();
        assert!(error.message.contains(needle), "{needle:?} not in {error}");
    };
    with_policy(
        &convert(
            Carrier::Baseline,
            Carrier::HostString,
            Carrier::HostString,
            Carrier::HostString,
        ),
        "reads v1 as Baseline but it carries HostString",
    );
    with_policy(
        &convert(
            Carrier::Baseline,
            Carrier::HostString,
            Carrier::Baseline,
            Carrier::Baseline,
        ),
        "writes v4 as HostString but it carries Baseline",
    );
    with_policy(
        &convert(
            Carrier::Baseline,
            Carrier::Baseline,
            Carrier::Baseline,
            Carrier::Baseline,
        ),
        "meaningless",
    );
    with_policy(
        &convert(
            Carrier::Baseline,
            Carrier::HostString,
            Carrier::Baseline,
            Carrier::HostString,
        ),
        "passes v4 as HostString to v2 as Baseline",
    );
    with_policy(
        &|f| f.values[2].carrier = Carrier::HostString,
        "as Baseline to v2 as HostString",
    );
    {
        let mut func = good.clone();
        convert(
            Carrier::Baseline,
            Carrier::HostString,
            Carrier::Baseline,
            Carrier::HostString,
        )(&mut func);
        let restored = ValueId(func.values.len() as u32);
        func.values.push(ValueDef {
            ty: I32,
            carrier: Carrier::Baseline,
            origin: ORIGIN,
        });
        func.blocks[2].ops.push(Op {
            result: Some(restored),
            origin: ORIGIN,
            kind: OpKind::Convert {
                v: ValueId(4),
                from: Carrier::HostString,
                to: Carrier::Baseline,
            },
        });
        func.blocks[2].term = RTerm::Goto(RBlockId(3), vec![restored]);
        func.verify_carriers(table, policy).unwrap();
    }
    expect(&|f| f.params.clear(), "entry block parameters");
    expect(
        &|f| f.blocks[1].term = RTerm::Goto(RBlockId(3), Vec::new()),
        "passes 0 arguments to 1",
    );
    expect(
        &|f| f.blocks[1].term = RTerm::Goto(RBlockId(9), Vec::new()),
        "missing block",
    );
    expect(
        &|f| {
            if let RTerm::Switch { cases, .. } = &mut f.blocks[0].term {
                cases.push((1, RBlockId(2), Vec::new()));
            }
        },
        "duplicate case",
    );
    expect(
        &|f| {
            if let RTerm::Switch { scrut_ty, .. } = &mut f.blocks[0].term {
                *scrut_ty = BOOL;
            }
        },
        "switch scrutinee",
    );
    // A second definition of an existing value.
    expect(
        &|f| {
            let existing = f.blocks[1].ops[0].result;
            f.blocks[2].ops[0].result = existing;
        },
        "defined twice",
    );
    // A block argument of the wrong type.
    expect(
        &|f| {
            let bad = f.values.len() as u32;
            f.values.push(ValueDef {
                ty: BOOL,
                carrier: Carrier::Baseline,
                origin: ORIGIN,
            });
            f.blocks[2].ops.push(Op {
                result: Some(ValueId(bad)),
                origin: ORIGIN,
                kind: OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
            });
            f.blocks[2].term = RTerm::Goto(RBlockId(3), vec![ValueId(bad)]);
        },
        "block argument",
    );
    // A cell used before its alloc.
    expect(
        &|f| {
            f.cells.push(CellDef {
                ty: I32,
                boxed: true,
                origin: ORIGIN,
            });
            let v = f.values.len() as u32;
            f.values.push(ValueDef {
                ty: I32,
                carrier: Carrier::Baseline,
                origin: ORIGIN,
            });
            f.blocks[0].ops.push(Op {
                result: Some(ValueId(v)),
                origin: ORIGIN,
                kind: OpKind::CellLoad(CellId(0)),
            });
        },
        "before being allocated",
    );
    // A borrow of a direct SSA value.
    expect(
        &|f| {
            let v = f.values.len() as u32;
            f.values.push(ValueDef {
                ty: REF_I32,
                carrier: Carrier::Baseline,
                origin: ORIGIN,
            });
            f.blocks[1].ops.push(Op {
                result: Some(ValueId(v)),
                origin: ORIGIN,
                kind: OpKind::BorrowView {
                    root: Root::Value(ValueId(1)),
                    path: Vec::new(),
                    mutable: false,
                },
            });
        },
        "address of a direct value",
    );
    // An intrinsic through Call.
    expect(
        &|f| {
            f.blocks[1].ops.push(Op {
                result: None,
                origin: ORIGIN,
                kind: OpKind::Call {
                    callee: Callee::Intrinsic {
                        name: StrId(0),
                        types: Vec::new(),
                        fallback: None,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
            });
        },
        "Callee::Intrinsic",
    );
    // A value op without a result.
    expect(&|f| f.blocks[1].ops[0].result = None, "must define a value");
    // Return type disagreement.
    expect(
        &|f| f.blocks[3].term = RTerm::Return(Some(ValueId(0))),
        "return",
    );
}

#[test]
fn repr_round_trips_through_postcard() {
    let func = enum_func();
    let bytes = postcard::to_stdvec(&func).unwrap();
    let back: Func = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(back, func);
}

#[test]
fn repr_copy_and_move_preserve_carriers() {
    let interns = world();
    let policy = CarrierPolicy::BASELINE.with(Carrier::SliceView);
    for moved in [false, true] {
        for carrier in [Carrier::Baseline, Carrier::SliceView] {
            let mut builder = FuncBuilder::new(item(1), signature(vec![REF_SLICE], REF_SLICE));
            let source = builder.param(REF_SLICE, ORIGIN);
            let op = if moved {
                OpKind::Move(source)
            } else {
                OpKind::Copy(source)
            };
            let result = builder.op(FuncBuilder::ENTRY, op, REF_SLICE, ORIGIN);
            builder.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(result)));
            let mut func = builder.finish();
            func.values[source.0 as usize].carrier = carrier;
            func.values[result.0 as usize].carrier = carrier;
            func.verify_carriers(&interns.types, policy).unwrap();

            func.values[result.0 as usize].carrier = if carrier == Carrier::Baseline {
                Carrier::SliceView
            } else {
                Carrier::Baseline
            };
            let error = func.verify_carriers(&interns.types, policy).unwrap_err();
            assert!(error.message.contains("use Convert"), "{error}");
        }
    }
}

#[test]
fn repr_direct_callees_share_heap_budget_and_failures() {
    let mut interns = world();
    let name = interns.intern_str("callee");
    let make_caller = |ty| {
        let mut b = FuncBuilder::new(item(1), signature(vec![ty], ty));
        let arg = b.param(ty, ORIGIN);
        let result = b.op(
            FuncBuilder::ENTRY,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: name,
                },
                function: None,
                args: vec![arg],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            ty,
            ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(result)));
        b.finish()
    };
    let mut b = FuncBuilder::new(item(2), signature(vec![REF_I32], REF_I32));
    let pointer = b.param(REF_I32, ORIGIN);
    let value = b.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Scalar { bits: 42, bytes: 4 }),
        I32,
        ORIGIN,
    );
    b.effect(
        FuncBuilder::ENTRY,
        OpKind::StoreThrough {
            ptr: pointer,
            v: value,
        },
        ORIGIN,
    );
    b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(pointer)));
    let callee = b.finish();
    let caller = make_caller(REF_I32);
    caller.verify(&interns.types).unwrap();
    callee.verify(&interns.types).unwrap();
    let hook = |_: &DirectItemRef, _: Vec<Value>| Ok(i32_value(99));
    let hooks = Hooks::new(&hook);
    let callees = BTreeMap::from([(name, &callee)]);
    let mut heap = Heap::new();
    let location = Location::root(heap.alloc(i32_value(1)));
    let pointer = Value::Ptr(location.clone());
    let mut interpreter =
        ReprInterpreter::new(&caller, &interns.types, &interns, &hooks, &mut heap)
            .with_callees(&callees);
    assert_eq!(interpreter.run(vec![pointer.clone()]).unwrap(), pointer);
    assert!(
        matches!(interpreter.trace.last(), Some(Event::Call { result, .. }) if result == &Ok(pointer.clone()))
    );
    assert_eq!(heap.read(&location).unwrap(), i32_value(42));

    // A value-only hook can return the pointer but cannot perform the callee's write.
    let mut control = Heap::new();
    let old_location = Location::root(control.alloc(i32_value(1)));
    let forward = |_: &DirectItemRef, args: Vec<Value>| Ok(args[0].clone());
    let old_hooks = Hooks::new(&forward);
    let mut old = ReprInterpreter::new(&caller, &interns.types, &interns, &old_hooks, &mut control);
    assert_eq!(
        old.run(vec![Value::Ptr(old_location.clone())]).unwrap(),
        Value::Ptr(old_location.clone())
    );
    assert_eq!(control.read(&old_location).unwrap(), i32_value(1));

    let missing = BTreeMap::new();
    let mut interpreter =
        ReprInterpreter::new(&caller, &interns.types, &interns, &hooks, &mut heap)
            .with_callees(&missing);
    assert_eq!(
        interpreter.run(vec![pointer.clone()]),
        Err(Trap::Hook("callee body unavailable".to_owned()))
    );

    let mut throwing = callee.clone();
    throwing.blocks[0].term = RTerm::Throw(None);
    let callees = BTreeMap::from([(name, &throwing)]);
    let mut interpreter =
        ReprInterpreter::new(&caller, &interns.types, &interns, &hooks, &mut heap)
            .with_callees(&callees);
    assert_eq!(interpreter.run(vec![pointer.clone()]), Err(Trap::Unwind));
    assert_eq!(
        interpreter.trace.last(),
        Some(&Event::Call {
            callee: DirectItemRef {
                item: item(2),
                link_name: name
            },
            args: vec![pointer],
            result: Err(Trap::Unwind),
        })
    );

    let scalar_caller = make_caller(I32);
    let mut b = FuncBuilder::new(item(2), signature(vec![I32], I32));
    let arg = b.param(I32, ORIGIN);
    b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(arg)));
    let identity = b.finish();
    let callees = BTreeMap::from([(name, &identity)]);
    let mut interpreter =
        ReprInterpreter::new(&scalar_caller, &interns.types, &interns, &hooks, &mut heap)
            .with_callees(&callees);
    assert_eq!(interpreter.run(vec![i32_value(17)]).unwrap(), i32_value(17));
    let mut old = ReprInterpreter::new(&scalar_caller, &interns.types, &interns, &hooks, &mut heap);
    assert_eq!(old.run(vec![i32_value(17)]).unwrap(), i32_value(99));

    let recursive = make_caller(I32);
    let callees = BTreeMap::from([(name, &recursive)]);
    let mut hooks = Hooks::new(&hook);
    hooks.step_limit = 10;
    let mut interpreter =
        ReprInterpreter::new(&recursive, &interns.types, &interns, &hooks, &mut heap)
            .with_callees(&callees);
    assert_eq!(interpreter.run(vec![i32_value(1)]), Err(Trap::Steps));
    hooks.step_limit = 1_000_000;
    let mut interpreter =
        ReprInterpreter::new(&recursive, &interns.types, &interns, &hooks, &mut heap)
            .with_callees(&callees);
    assert_eq!(
        interpreter.run(vec![i32_value(1)]),
        Err(Trap::Unsupported("direct-call depth limit"))
    );
}

#[test]
fn repr_direct_calls_resume_loop_parameters_and_void_calls() {
    let mut interns = world();
    let name = interns.intern_str("increment");
    let mut b = FuncBuilder::new(item(2), signature(vec![I32], I32));
    let arg = b.param(I32, ORIGIN);
    let one = b.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let sum = b.op(
        FuncBuilder::ENTRY,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: arg,
            r: one,
            ty: I32,
        },
        I32,
        ORIGIN,
    );
    b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(sum)));
    let callee = b.finish();
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let limit = b.param(I32, ORIGIN);
    let zero = b.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let header = b.block();
    let counter = b.block_param(header, I32, ORIGIN);
    let body = b.block();
    let exit = b.block();
    b.terminate(FuncBuilder::ENTRY, RTerm::Goto(header, vec![zero]));
    let more = b.op(
        header,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: counter,
            r: limit,
            ty: I32,
        },
        BOOL,
        ORIGIN,
    );
    b.terminate(
        header,
        RTerm::Switch {
            scrut: more,
            scrut_ty: BOOL,
            cases: vec![(0, exit, Vec::new())],
            default: (body, Vec::new()),
        },
    );
    let call = || OpKind::Call {
        callee: Callee::Direct {
            item: item(2),
            link_name: name,
        },
        function: None,
        args: vec![counter],
        track_caller: None,
        semantic: CallSemantic::Ordinary,
        host: None,
    };
    b.effect(body, call(), ORIGIN);
    let next = b.op(body, call(), I32, ORIGIN);
    b.terminate(body, RTerm::Goto(header, vec![next]));
    b.terminate(exit, RTerm::Return(Some(counter)));
    let caller = b.finish();
    caller.verify(&interns.types).unwrap();
    callee.verify(&interns.types).unwrap();
    let callees = BTreeMap::from([(name, &callee)]);
    let hook = |_: &DirectItemRef, _: Vec<Value>| Err(Trap::Hook("hook must not run".to_owned()));
    let hooks = Hooks::new(&hook);
    for n in [0, 1, 3, 16] {
        let mut heap = Heap::new();
        let mut interpreter =
            ReprInterpreter::new(&caller, &interns.types, &interns, &hooks, &mut heap)
                .with_callees(&callees);
        assert_eq!(interpreter.run(vec![i32_value(n)]).unwrap(), i32_value(n));
        assert_eq!(
            interpreter
                .trace
                .iter()
                .filter(|event| matches!(event, Event::Call { .. }))
                .count(),
            2 * n as usize
        );
    }
}

#[test]
fn repr_allows_move_of_loop_redefined_value_and_block_param_across_backedge() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let limit = b.param(I32, ORIGIN);
    let zero = b.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let header = b.block();
    let counter = b.block_param(header, I32, ORIGIN);
    let exit = b.block();
    let exit_val = b.block_param(exit, I32, ORIGIN);
    b.terminate(FuncBuilder::ENTRY, RTerm::Goto(header, vec![zero]));

    let body_val = b.op(header, OpKind::Copy(counter), I32, ORIGIN);
    let moved_param = b.op(header, OpKind::Move(counter), I32, ORIGIN);
    let moved_val = b.op(header, OpKind::Move(body_val), I32, ORIGIN);
    let more = b.op(
        header,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: moved_param,
            r: limit,
            ty: I32,
        },
        BOOL,
        ORIGIN,
    );
    b.terminate(
        header,
        RTerm::Switch {
            scrut: more,
            scrut_ty: BOOL,
            cases: vec![(0, exit, vec![moved_val])],
            default: (header, vec![moved_val]),
        },
    );
    b.terminate(exit, RTerm::Return(Some(exit_val)));
    let func = b.finish();
    func.verify(&interns.types).unwrap();
}

#[test]
fn repr_rejects_move_of_loop_invariant_outer_value_across_backedge() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![I32, BOOL], I32));
    let outer = b.param(I32, ORIGIN);
    let cond = b.param(BOOL, ORIGIN);
    let header = b.block();
    let exit = b.block();
    let exit_val = b.block_param(exit, I32, ORIGIN);
    b.terminate(FuncBuilder::ENTRY, RTerm::Goto(header, Vec::new()));

    let moved = b.op(header, OpKind::Move(outer), I32, ORIGIN);
    b.terminate(
        header,
        RTerm::Switch {
            scrut: cond,
            scrut_ty: BOOL,
            cases: vec![(0, exit, vec![moved])],
            default: (header, Vec::new()),
        },
    );
    b.terminate(exit, RTerm::Return(Some(exit_val)));
    let func = b.finish();
    let error = func.verify(&interns.types).unwrap_err();
    assert!(error.message.contains("after being moved"), "{error}");
}
