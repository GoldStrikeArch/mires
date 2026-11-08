//! Unit tests: hand-built `Func`s and `Plan`s through `js::function::lower`, printed and compared
//! to the shapes v1's goldens pin (`examples/emit/*.js.expected`).

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::item::JsName;
use rcgjs_object::jir::interns::{Interns, StrId};
use rcgjs_object::jir::repr::{
    Func, FuncBuilder, OpKind, Proj, RAssertMessage, RBlockId, RTerm, Root, ValueId,
};
use rcgjs_object::jir::semantic::{
    AbiSignature, BinaryOp, Body, BodyKind, BodySemantic, CallSemantic, Callee, CastKind,
    ConstTree, DefinitionId, DirectItemRef, DropGlue, HostPayload, ItemId, KeyedListCallable,
    KeyedListPayload, Signature, UnaryOp, VtableEntry, KEYED_LIST_FORMAT_V1,
};
use rcgjs_object::jir::summary::{Availability, ItemClass, ItemSummary, SemanticKey};
use rcgjs_object::jir::types::{
    EnumRepr, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts,
    NominalDiscriminant, NominalForm, PointerForm, TypeDescriptor, TypeId, TypeKind, TypeMeta,
    VariantDescriptor,
};
use rcgjs_object::jsast;
use rcgjs_object::object::{BlobRef, ItemKind, Linkage};
use rcgjs_object::opts::Opts;
use structurizer::{Region, Scrut, Test};

use super::super::plan::{
    self, BlockPayload, CopyOperand, EdgeCopy, OpRef, Plan, RScrut, NO_ORIGIN,
};
use super::super::LowerInput;
use super::function;

const UNIT: TypeId = TypeId(1);
const BOOL: TypeId = TypeId(2);
const I32: TypeId = TypeId(3);
const U8: TypeId = TypeId(4);
const USIZE: TypeId = TypeId(5);
const POINT: TypeId = TypeId(6);
const ARRAY: TypeId = TypeId(7);
const SLICE: TypeId = TypeId(8);
const REF_POINT: TypeId = TypeId(9);
const REF_I32: TypeId = TypeId(10);
const REF_SLICE: TypeId = TypeId(11);
const REF_ARRAY: TypeId = TypeId(12);
const OVERFLOW: TypeId = TypeId(13);
const SHAPE: TypeId = TypeId(14);
const SIGN: TypeId = TypeId(15);
const LETTER: TypeId = TypeId(16);
const U32: TypeId = TypeId(17);
const OPTION: TypeId = TypeId(18);
const I64: TypeId = TypeId(19);
const F32: TypeId = TypeId(20);
const STR_REF: TypeId = TypeId(21);
const STR: TypeId = TypeId(22);
const LOCATION: TypeId = TypeId(23);
const REF_LOCATION: TypeId = TypeId(24);
const RAW_I32: TypeId = TypeId(25);
const RAW_POINT: TypeId = TypeId(26);
const CHAR: TypeId = TypeId(27);
const ISIZE: TypeId = TypeId(28);
const OPTION_REF: TypeId = TypeId(29);
const DYNAMIC: TypeId = TypeId(30);
const REF_DYNAMIC: TypeId = TypeId(31);

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
    let mut d = descriptor(id, TypeKind::Integer { signed, bits }, u64::from(bits / 8));
    d.js.int = Some(IntFacts {
        signed,
        bits,
        repr: if bits > 32 {
            IntRepr::BigInt
        } else {
            IntRepr::Number
        },
    });
    d.js.byte_element = bits == 8;
    d
}

fn field(key: FieldKey, ty: TypeId) -> FieldDescriptor {
    FieldDescriptor { key, ty, offset: 0 }
}

fn variant(
    name: StrId,
    fields: Vec<FieldDescriptor>,
    discr: i128,
    tag: Option<u128>,
) -> VariantDescriptor {
    VariantDescriptor {
        name,
        fields,
        logical_discr: discr,
        physical_tag: tag.map(|bits| NominalDiscriminant { bits, bytes: 4 }),
        result_ok: false,
        saved_local_keys: Vec::new(),
    }
}

fn pointer(id: TypeId, pointee: TypeId, form: PointerForm, raw: bool) -> TypeDescriptor {
    let kind = if raw {
        TypeKind::RawPointer {
            pointee,
            mutable: true,
        }
    } else {
        TypeKind::Reference {
            pointee,
            mutable: true,
        }
    };
    let mut d = descriptor(id, kind, 4);
    d.js.pointer_form = Some(form);
    d
}

fn def(n: u64) -> DefinitionId {
    DefinitionId {
        crate_id: 1,
        local_hash: n,
    }
}

fn world() -> Interns {
    let mut interns = Interns::new();
    let s = |interns: &mut Interns, text: &str| interns.intern_str(text);
    let (x, y, f0, f1, w, h) = (
        s(&mut interns, "x"),
        s(&mut interns, "y"),
        s(&mut interns, "_0"),
        s(&mut interns, "_1"),
        s(&mut interns, "w"),
        s(&mut interns, "h"),
    );
    let point = s(&mut interns, "Point");
    let (nothing, num, pair, named) = (
        s(&mut interns, "Nothing"),
        s(&mut interns, "Num"),
        s(&mut interns, "Pair"),
        s(&mut interns, "Named"),
    );
    let (neg, zero, pos) = (
        s(&mut interns, "Neg"),
        s(&mut interns, "Zero"),
        s(&mut interns, "Pos"),
    );
    let (a, b) = (s(&mut interns, "A"), s(&mut interns, "B"));
    let (none, some) = (s(&mut interns, "None"), s(&mut interns, "Some"));
    let (location, file, line, column) = (
        s(&mut interns, "Location"),
        s(&mut interns, "file"),
        s(&mut interns, "line"),
        s(&mut interns, "column"),
    );

    interns.intern_type(descriptor(UNIT, TypeKind::Tuple(Vec::new()), 0));
    interns.intern_type(descriptor(BOOL, TypeKind::Bool, 1));
    interns.intern_type(descriptor(CHAR, TypeKind::Char, 4));
    interns.intern_type(int(I32, true, 32));
    interns.intern_type(int(U8, false, 8));
    interns.intern_type(int(USIZE, false, 32));
    interns.intern_type(int(ISIZE, true, 32));
    interns.intern_type(int(U32, false, 32));
    interns.intern_type(int(I64, true, 64));
    interns.intern_type(descriptor(F32, TypeKind::Float { bits: 32 }, 4));

    let mut point_ty = descriptor(
        POINT,
        TypeKind::Adt {
            def: def(11),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                point,
                vec![field(FieldKey::Name(x), I32), field(FieldKey::Name(y), I32)],
                0,
                None,
            )],
            niche: None,
        },
        8,
    );
    point_ty.js.is_indirect = true;
    point_ty.js.needs_clone = true;
    point_ty.js.fields = vec![field(FieldKey::Name(x), I32), field(FieldKey::Name(y), I32)];
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
    array.js.fields = vec![field(FieldKey::Index(0), I32)];
    interns.intern_type(array);

    let mut slice = descriptor(SLICE, TypeKind::Slice(I32), 0);
    slice.layout.sized = false;
    slice.layout.size = None;
    slice.layout.zst = false;
    slice.js.is_indirect = true;
    interns.intern_type(slice);

    let mut str_ty = descriptor(STR, TypeKind::Str, 0);
    str_ty.layout.sized = false;
    str_ty.layout.size = None;
    str_ty.layout.zst = false;
    str_ty.js.is_indirect = true;
    interns.intern_type(str_ty);

    interns.intern_type(pointer(REF_POINT, POINT, PointerForm::Thin, false));
    interns.intern_type(pointer(REF_I32, I32, PointerForm::Slot, false));
    interns.intern_type(pointer(REF_SLICE, SLICE, PointerForm::FatSlice, false));
    interns.intern_type(pointer(REF_ARRAY, ARRAY, PointerForm::Thin, false));
    interns.intern_type(pointer(STR_REF, STR, PointerForm::Str, false));
    interns.intern_type(pointer(RAW_I32, I32, PointerForm::Slot, true));
    interns.intern_type(pointer(RAW_POINT, POINT, PointerForm::Slot, true));

    let mut dynamic = descriptor(
        DYNAMIC,
        TypeKind::Dynamic {
            principal: Some(def(18)),
            auto: Vec::new(),
        },
        0,
    );
    dynamic.layout.sized = false;
    dynamic.layout.size = None;
    dynamic.layout.zst = false;
    dynamic.js.is_indirect = true;
    interns.intern_type(dynamic);
    interns.intern_type(pointer(REF_DYNAMIC, DYNAMIC, PointerForm::FatDyn, false));

    let mut overflow = descriptor(OVERFLOW, TypeKind::Tuple(vec![U32, BOOL]), 8);
    overflow.js.is_indirect = true;
    overflow.js.needs_clone = true;
    overflow.js.fields = vec![
        field(FieldKey::Index(0), U32),
        field(FieldKey::Index(1), BOOL),
    ];
    interns.intern_type(overflow);

    let mut shape = descriptor(
        SHAPE,
        TypeKind::Adt {
            def: def(12),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(nothing, Vec::new(), 0, Some(0)),
                variant(num, vec![field(FieldKey::Name(f0), I32)], 1, Some(1)),
                variant(
                    pair,
                    vec![
                        field(FieldKey::Name(f0), I32),
                        field(FieldKey::Name(f1), I32),
                    ],
                    2,
                    Some(2),
                ),
                variant(
                    named,
                    vec![field(FieldKey::Name(w), I32), field(FieldKey::Name(h), I32)],
                    3,
                    Some(3),
                ),
            ],
            niche: None,
        },
        12,
    );
    shape.js.is_indirect = true;
    shape.js.needs_clone = true;
    shape.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(shape);

    let mut sign = descriptor(
        SIGN,
        TypeKind::Adt {
            def: def(13),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(neg, Vec::new(), 0, Some(0)),
                variant(zero, Vec::new(), 1, Some(1)),
                variant(pos, Vec::new(), 2, Some(2)),
            ],
            niche: None,
        },
        1,
    );
    sign.js.enum_repr = Some(EnumRepr::Tag { fieldless: true });
    sign.js.is_primitive_enum = true;
    interns.intern_type(sign);

    let mut letter = descriptor(
        LETTER,
        TypeKind::Adt {
            def: def(14),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(a, Vec::new(), 65, Some(65)),
                variant(b, Vec::new(), 66, Some(66)),
            ],
            niche: None,
        },
        1,
    );
    letter.js.enum_repr = Some(EnumRepr::Number {
        bits: 8,
        signed: false,
    });
    letter.js.is_primitive_enum = true;
    letter.js.int = None;
    interns.intern_type(letter);

    let mut option = descriptor(
        OPTION,
        TypeKind::Adt {
            def: def(15),
            args: vec![I32],
            form: NominalForm::Enum,
            variants: vec![
                variant(none, Vec::new(), 0, Some(0)),
                variant(some, vec![field(FieldKey::Name(f0), I32)], 1, Some(1)),
            ],
            niche: None,
        },
        8,
    );
    option.js.is_indirect = true;
    option.js.needs_clone = true;
    option.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(option);

    let mut option_ref = descriptor(
        OPTION_REF,
        TypeKind::Adt {
            def: def(17),
            args: vec![REF_I32],
            form: NominalForm::Enum,
            variants: vec![
                variant(none, Vec::new(), 0, None),
                variant(some, vec![field(FieldKey::Name(f0), REF_I32)], 1, None),
            ],
            niche: Some(rcgjs_object::jir::types::Niche {
                untagged_variant: 1,
                niche_field: vec![0],
                niche_start: 0,
                valid_range: (0, u32::MAX as u128),
            }),
        },
        4,
    );
    option_ref.js.is_indirect = true;
    option_ref.js.needs_clone = true;
    option_ref.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(option_ref);

    let mut loc = descriptor(
        LOCATION,
        TypeKind::Adt {
            def: def(16),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                location,
                vec![
                    field(FieldKey::Name(file), STR_REF),
                    field(FieldKey::Name(line), U32),
                    field(FieldKey::Name(column), U32),
                ],
                0,
                None,
            )],
            niche: None,
        },
        12,
    );
    loc.js.is_indirect = true;
    loc.js.needs_clone = true;
    interns.intern_type(loc);
    interns.intern_type(pointer(REF_LOCATION, LOCATION, PointerForm::Thin, false));
    interns
}

fn item(n: u64) -> ItemId {
    ItemId {
        definition: def(n),
        instantiation: n as u128,
    }
}

fn signature(arguments: Vec<TypeId>, result: TypeId) -> Signature {
    Signature {
        abi: AbiSignature {
            parameters: arguments.len() as u16,
            spread: None,
            caller_location: false,
        },
        arguments,
        result,
    }
}

fn summary(deps: &[&str]) -> ItemSummary {
    ItemSummary {
        key: SemanticKey {
            link_name: "f".to_owned(),
            item: None,
        },
        body_hash: None,
        semantic_hash: None,
        availability: Availability::Available,
        deps: deps.iter().map(|d| (*d).to_owned()).collect(),
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
    }
}

fn body_of(kind: BodyKind, sig: Signature) -> Body {
    Body {
        item: item(1),
        kind,
        semantic: BodySemantic::Ordinary,
        signature: sig,
        locals: Vec::new(),
        blocks: Vec::new(),
    }
}

/// A builder-side helper: the whole of a block's ops as a payload.
fn payload(func: &Func, block: u32, edge: Option<(u32, u32)>) -> BlockPayload {
    BlockPayload {
        block: Some(RBlockId(block)),
        ops: (0..func.blocks[block as usize].ops.len())
            .map(|index| OpRef {
                block: RBlockId(block),
                index: index as u32,
            })
            .collect(),
        edge: edge.map(|(a, b)| (RBlockId(a), RBlockId(b))),
    }
}

/// A plan naming `named` and inlining every other value.
fn plan_of(func: &Func, named: &[(u32, &str)]) -> Plan {
    let mut plan = Plan::default();
    for (v, name) in named {
        plan.names.insert(ValueId(*v), (*name).to_owned());
    }
    for (index, _) in func.values.iter().enumerate() {
        let v = ValueId(index as u32);
        match plan.names.entry(v) {
            std::collections::btree_map::Entry::Occupied(_) => {
                if !func.params.contains(&v) {
                    plan.declared_order.push(v);
                }
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(format!("_{index}"));
                plan.inline.insert(v);
            }
        }
    }
    plan
}

fn lower(
    interns: &Interns,
    name: &str,
    deps: &[&str],
    body: &Body,
    func: &Func,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
) -> String {
    lower_retargeted(
        interns,
        name,
        deps,
        body,
        func,
        plan,
        region,
        crate::lower::EMPTY_CALL_RETARGETS,
    )
}

#[allow(clippy::too_many_arguments)]
fn lower_retargeted(
    interns: &Interns,
    name: &str,
    deps: &[&str],
    body: &Body,
    func: &Func,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
    call_retargets: &crate::lower::CallRetargets,
) -> String {
    let opts = Opts::default();
    let name = JsName::new(name.to_owned());
    let summary = summary(deps);
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body,
        origins: None,
        interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets,
    };
    match function::lower(&input, func, plan, region) {
        Ok(item) => jsast::stmt_to_string(&item.decl),
        Err(decline) => panic!("declined: {decline}"),
    }
}

fn straight(func: &Func) -> Region<BlockPayload, RScrut> {
    Region::Seq(vec![Region::Block(payload(func, 0, None)), Region::Return])
}

#[test]
fn arith_mix_matches_the_01_arith_golden() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![I32, I32], I32));
    let a = b.param(I32, NO_ORIGIN);
    let bb = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let sum = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: a,
            r: bb,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    let three = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 3, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let mul = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Mul,
            l: sum,
            r: three,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    let two = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 2, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let shl = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Shl,
            l: mul,
            r: two,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    let xor = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::BitXor,
            l: shl,
            r: a,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(xor)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "a"), (1, "b")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "mix", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function mix(a, b) {\n  return ((Math.imul(a + b | 0, 3) | 0) << 2 | 0) ^ a | 0;\n}\n"
    );
}

#[test]
fn u8_arith_masks_and_shift_amounts_match_the_golden() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![U8, U8], U8));
    let x = b.param(U8, NO_ORIGIN);
    let y = b.param(U8, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let sum = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: x,
            r: y,
            ty: U8,
        },
        U8,
        NO_ORIGIN,
    );
    let three = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 3, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let shl = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Shl,
            l: sum,
            r: three,
            ty: U8,
        },
        U8,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(shl)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "x"), (1, "y")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "bytes", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function bytes(x, y) {\n  return (x + y & 255) << (3 & 7) & 255;\n}\n"
    );
}

#[test]
fn overflowing_add_is_the_arrow_form() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![U32, U32], OVERFLOW));
    let a = b.param(U32, NO_ORIGIN);
    let c = b.param(U32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let r = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::AddWithOverflow,
            l: a,
            r: c,
            ty: U32,
        },
        OVERFLOW,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(r)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "a"), (1, "b")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(a, b) {\n  return (($w) => [$w >>> 0, $w >>> 0 !== $w])(a + b);\n}\n"
    );
}

#[test]
fn bigint_and_float_and_casts_spell_as_v1() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![I64, I32, F32], I64));
    let a = b.param(I64, NO_ORIGIN);
    let n = b.param(I32, NO_ORIGIN);
    let f = b.param(F32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    // `a << n` with a number amount on a BigInt.
    let shl = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Shl,
            l: a,
            r: n,
            ty: I64,
        },
        I64,
        NO_ORIGIN,
    );
    // `n as i64`.
    let widened = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::IntToInt,
            v: n,
            from: I32,
            to: I64,
        },
        I64,
        NO_ORIGIN,
    );
    let sum = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: shl,
            r: widened,
            ty: I64,
        },
        I64,
        NO_ORIGIN,
    );
    // `f * f` at f32, `as i64`.
    let sq = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Mul,
            l: f,
            r: f,
            ty: F32,
        },
        F32,
        NO_ORIGIN,
    );
    let fi = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::FloatToInt,
            v: sq,
            from: F32,
            to: I64,
        },
        I64,
        NO_ORIGIN,
    );
    let total = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Sub,
            l: sum,
            r: fi,
            ty: I64,
        },
        I64,
        NO_ORIGIN,
    );
    let neg = b.op(
        e,
        OpKind::Unary {
            op: UnaryOp::Neg,
            v: total,
            ty: I64,
        },
        I64,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(neg)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "a"), (1, "n"), (2, "f")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "g", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function g(a, n, f) {\n  return BigInt.asIntN(64, -BigInt.asIntN(64, BigInt.asIntN(64, BigInt.asIntN(64, a << (BigInt(n) & 63n)) + BigInt.asIntN(64, BigInt(n))) - __rt.f2i_big(Math.fround(f * f), 64, true)));\n}\n"
    );
}

#[test]
fn enum_switch_matches_the_18_enum_shapes_golden() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![SHAPE], I32));
    let s = b.param(SHAPE, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let d = b.op(e, OpKind::Discriminant(s), ISIZE, NO_ORIGIN);
    let (b1, b2, b3, b4, b5, b6) = (
        b.block(),
        b.block(),
        b.block(),
        b.block(),
        b.block(),
        b.block(),
    );
    let ret = b.block_param(b6, I32, NO_ORIGIN);
    b.terminate(
        e,
        RTerm::Switch {
            scrut: d,
            scrut_ty: ISIZE,
            cases: vec![
                (0, b1, vec![]),
                (1, b2, vec![]),
                (2, b3, vec![]),
                (3, b4, vec![]),
            ],
            default: (b5, vec![]),
        },
    );
    let zero = b.op(
        b1,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    b.terminate(b1, RTerm::Goto(b6, vec![zero]));
    let n = b.op(
        b2,
        OpKind::Field {
            base: s,
            variant: Some(1),
            field: 0,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(b2, RTerm::Goto(b6, vec![n]));
    let p0 = b.op(
        b3,
        OpKind::Field {
            base: s,
            variant: Some(2),
            field: 0,
        },
        I32,
        NO_ORIGIN,
    );
    let p1 = b.op(
        b3,
        OpKind::Field {
            base: s,
            variant: Some(2),
            field: 1,
        },
        I32,
        NO_ORIGIN,
    );
    let sum = b.op(
        b3,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: p0,
            r: p1,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(b3, RTerm::Goto(b6, vec![sum]));
    let w = b.op(
        b4,
        OpKind::Field {
            base: s,
            variant: Some(3),
            field: 0,
        },
        I32,
        NO_ORIGIN,
    );
    let h = b.op(
        b4,
        OpKind::Field {
            base: s,
            variant: Some(3),
            field: 1,
        },
        I32,
        NO_ORIGIN,
    );
    let area = b.op(
        b4,
        OpKind::Binary {
            op: BinaryOp::Mul,
            l: w,
            r: h,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(b4, RTerm::Goto(b6, vec![area]));
    b.terminate(b5, RTerm::Unreachable);
    b.terminate(b6, RTerm::Return(Some(ret)));
    let func = b.finish();
    let mut plan = plan_of(&func, &[(0, "s"), (ret.0, "_0")]);
    plan.tag_values.insert(d);
    for (from, src) in [(b1, zero), (b2, n), (b3, sum), (b4, area)] {
        plan.copies.insert(
            (from, b6),
            vec![EdgeCopy {
                dst: CopyOperand::Value(ret),
                src: CopyOperand::Value(src),
            }],
        );
    }
    let scrut = RScrut::Value {
        value: d,
        ty: ISIZE,
        tag_enum: Some(SHAPE),
    };
    let arm = |func: &Func, block: RBlockId, value: u128| {
        (
            vec![plan::encode(&interns.types, ISIZE, value)],
            Region::Seq(vec![
                Region::Block(payload(func, block.0, Some((block.0, b6.0)))),
                Region::Break(None),
            ]),
        )
    };
    let region = Region::Seq(vec![
        Region::Block(payload(&func, 0, None)),
        Region::Switch {
            scrut: Scrut::Payload(scrut),
            arms: vec![
                arm(&func, b1, 0),
                arm(&func, b2, 1),
                arm(&func, b3, 2),
                arm(&func, b4, 3),
            ],
            default: Box::new(Region::Unreachable),
            label: None,
        },
        Region::Block(payload(&func, b6.0, None)),
        Region::Return,
    ]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "area", &[], &body, &func, &plan, region);
    assert_eq!(
        js,
        "function area(s) {\n  let _0;\n  switch (s.TAG) {\n    case \"Nothing\":\n      _0 = 0;\n      break;\n    case \"Num\":\n      _0 = s._0;\n      break;\n    case \"Pair\":\n      _0 = s._0 + s._1 | 0;\n      break;\n    case \"Named\":\n      _0 = Math.imul(s.w, s.h) | 0;\n      break;\n    default:\n      throw __rt.abort_error(new Error(\"rustc_codegen_js: entered unreachable code\"));\n  }\n  return _0;\n}\n"
    );
}

#[test]
fn fieldless_tag_and_number_enums_switch_on_the_value() {
    let interns = world();
    // `is_not_zero(s: Sign) -> bool`: `switch (s) { case "Zero": .. case "Neg": case "Pos": .. }`.
    let mut b = FuncBuilder::new(item(1), signature(vec![SIGN], BOOL));
    let s = b.param(SIGN, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let d = b.op(e, OpKind::Discriminant(s), ISIZE, NO_ORIGIN);
    let (b1, b2, b3, b4) = (b.block(), b.block(), b.block(), b.block());
    let ret = b.block_param(b4, BOOL, NO_ORIGIN);
    b.terminate(
        e,
        RTerm::Switch {
            scrut: d,
            scrut_ty: ISIZE,
            cases: vec![(1, b1, vec![]), (0, b2, vec![]), (2, b2, vec![])],
            default: (b3, vec![]),
        },
    );
    let f = b.op(
        b1,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 1 }),
        BOOL,
        NO_ORIGIN,
    );
    b.terminate(b1, RTerm::Goto(b4, vec![f]));
    let t = b.op(
        b2,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 1 }),
        BOOL,
        NO_ORIGIN,
    );
    b.terminate(b2, RTerm::Goto(b4, vec![t]));
    b.terminate(b3, RTerm::Unreachable);
    b.terminate(b4, RTerm::Return(Some(ret)));
    let func = b.finish();
    let mut plan = plan_of(&func, &[(0, "s"), (ret.0, "_0")]);
    plan.tag_values.insert(d);
    for (from, src) in [(b1, f), (b2, t)] {
        plan.copies.insert(
            (from, b4),
            vec![EdgeCopy {
                dst: CopyOperand::Value(ret),
                src: CopyOperand::Value(src),
            }],
        );
    }
    let scrut = RScrut::Value {
        value: d,
        ty: ISIZE,
        tag_enum: Some(SIGN),
    };
    let enc = |v: u128| plan::encode(&interns.types, ISIZE, v);
    let region = Region::Seq(vec![
        Region::Block(payload(&func, 0, None)),
        Region::Switch {
            scrut: Scrut::Payload(scrut),
            arms: vec![
                (
                    vec![enc(1)],
                    Region::Seq(vec![
                        Region::Block(payload(&func, b1.0, Some((b1.0, b4.0)))),
                        Region::Break(None),
                    ]),
                ),
                (
                    vec![enc(0), enc(2)],
                    Region::Seq(vec![
                        Region::Block(payload(&func, b2.0, Some((b2.0, b4.0)))),
                        Region::Break(None),
                    ]),
                ),
            ],
            default: Box::new(Region::Unreachable),
            label: None,
        },
        Region::Block(payload(&func, b4.0, None)),
        Region::Return,
    ]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "is_not_zero", &[], &body, &func, &plan, region);
    assert_eq!(
        js,
        "function is_not_zero(s) {\n  let _0;\n  switch (s) {\n    case \"Zero\":\n      _0 = false;\n      break;\n    case \"Neg\":\n    case \"Pos\":\n      _0 = true;\n      break;\n    default:\n      throw __rt.abort_error(new Error(\"rustc_codegen_js: entered unreachable code\"));\n  }\n  return _0;\n}\n"
    );

    // A `Number` enum's discriminant is the value; a `Discriminant` not listed as a tag is the
    // number, which for a `Number` enum is the value too.
    let mut b = FuncBuilder::new(item(1), signature(vec![LETTER], I32));
    let l = b.param(LETTER, NO_ORIGIN);
    let d = b.op(e, OpKind::Discriminant(l), U8, NO_ORIGIN);
    let widened = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::IntToInt,
            v: d,
            from: U8,
            to: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(widened)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "l")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "num", &[], &body, &func, &plan, straight(&func));
    assert_eq!(js, "function num(l) {\n  return l | 0;\n}\n");
}

#[test]
fn if_and_while_regions_spell_tests_and_loops() {
    use rcgjs_object::jir::origin::{Origin, OriginId, OriginKind, OriginTable};

    let mut interns = world();
    let source = one_fact(&mut interns, "count.rs", 42, 4);
    let origins = OriginTable {
        origins: vec![Origin {
            kind: OriginKind::Statement,
            source: Some(source),
            local_name: None,
            inline_stack: Vec::new(),
        }],
        statements: Vec::new(),
        terminators: Vec::new(),
        locals: Vec::new(),
        item_path: interns.intern_str("count"),
        root: None,
    };
    // `fn count(n: i32) -> i32 { let mut i = 0; while i < n { i = i + 1 } i }`
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let n = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let zero = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let (head, body_block, exit) = (b.block(), b.block(), b.block());
    let i = b.block_param(head, I32, NO_ORIGIN);
    b.terminate(e, RTerm::Goto(head, vec![zero]));
    let lt = b.op(
        head,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: i,
            r: n,
            ty: I32,
        },
        BOOL,
        OriginId(0),
    );
    b.terminate(
        head,
        RTerm::Switch {
            scrut: lt,
            scrut_ty: BOOL,
            cases: vec![(0, exit, vec![])],
            default: (body_block, vec![]),
        },
    );
    let one = b.op(
        body_block,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let next = b.op(
        body_block,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: i,
            r: one,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(body_block, RTerm::Goto(head, vec![next]));
    b.terminate(exit, RTerm::Return(Some(i)));
    let func = b.finish();
    let mut plan = plan_of(&func, &[(0, "n"), (i.0, "i")]);
    plan.copies.insert(
        (e, head),
        vec![EdgeCopy {
            dst: CopyOperand::Value(i),
            src: CopyOperand::Value(zero),
        }],
    );
    plan.copies.insert(
        (body_block, head),
        vec![EdgeCopy {
            dst: CopyOperand::Value(i),
            src: CopyOperand::Value(next),
        }],
    );
    let region = || {
        Region::Seq(vec![
            Region::Block(payload(&func, 0, Some((0, head.0)))),
            Region::Loop {
                label: None,
                body: Box::new(Region::Seq(vec![
                    Region::Block(payload(&func, head.0, None)),
                    Region::If {
                        scrut: Scrut::Payload(RScrut::Value {
                            value: lt,
                            ty: BOOL,
                            tag_enum: None,
                        }),
                        test: Test::IsTrue,
                        then_: Box::new(Region::Seq(vec![
                            Region::Block(payload(
                                &func,
                                body_block.0,
                                Some((body_block.0, head.0)),
                            )),
                            Region::Continue(None),
                        ])),
                        else_: Box::new(Region::Break(None)),
                    },
                ])),
            },
            Region::Block(payload(&func, exit.0, None)),
            Region::Return,
        ])
    };
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "count", &[], &body, &func, &plan, region());
    assert_eq!(
        js,
        "function count(n) {\n  let i;\n  i = 0;\n  while (i < n) {\n    i = i + 1 | 0;\n    continue;\n  }\n  return i;\n}\n"
    );
    let opts = Opts {
        source_map: true,
        ..Opts::default()
    };
    let name = JsName::new("count".to_owned());
    let summary = summary(&[]);
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body: &body,
        origins: Some(&origins),
        interns: &interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    let lowered = function::lower(&input, &func, &plan, region()).expect("lowers with maps");
    let (mapped, mappings) =
        jsast::program_to_string_mapped(&[lowered.decl], BTreeMap::new(), None);
    assert_eq!(
        mapped, js,
        "source markers must not change the emitted program"
    );
    let condition = mappings
        .iter()
        .find(|mapping| {
            mapping.loc.file == "count.rs" && mapping.loc.line == 42 && mapping.loc.col == 4
        })
        .expect("the fused while condition retains its source");
    assert!(mapped
        .lines()
        .nth(condition.gen_line as usize)
        .unwrap()
        .contains("while (i < n)"));
}

#[test]
fn pointer_bump_and_nth_match_the_14_pointers_golden() {
    let interns = world();
    // `bump(target: &mut i32, by: i32) { *target = *target + by }`
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_I32, I32], UNIT));
    let target = b.param(REF_I32, NO_ORIGIN);
    let by = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let read = b.op(e, OpKind::Deref(target), I32, NO_ORIGIN);
    let sum = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: read,
            r: by,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::StoreThrough {
            ptr: target,
            v: sum,
        },
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(None));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "target"), (1, "by")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "bump", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function bump(target, by) {\n  target.buf[target.off] = target.buf[target.off] + by | 0;\n}\n"
    );

    // `nth(base: *const i32, index: usize) -> i32 { *base.add(index) }`
    let mut b = FuncBuilder::new(item(1), signature(vec![RAW_I32, USIZE], I32));
    let base = b.param(RAW_I32, NO_ORIGIN);
    let index = b.param(USIZE, NO_ORIGIN);
    let moved = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Offset,
            l: base,
            r: index,
            ty: RAW_I32,
        },
        RAW_I32,
        NO_ORIGIN,
    );
    let read = b.op(e, OpKind::Deref(moved), I32, NO_ORIGIN);
    b.terminate(e, RTerm::Return(Some(read)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "base"), (1, "index")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "nth", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function nth(base, index) {\n  return base.buf[base.off + index];\n}\n"
    );
}

#[test]
fn a_private_version_retargets_only_the_direct_call_spelling() {
    let mut interns = world();
    let original = interns.intern_str("callee$h1");
    let target = item(2);
    let mut b = FuncBuilder::new(item(1), signature(Vec::new(), UNIT));
    let entry = FuncBuilder::ENTRY;
    b.effect(
        entry,
        OpKind::Call {
            callee: Callee::Direct {
                item: target,
                link_name: original,
            },
            function: None,
            args: Vec::new(),
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        NO_ORIGIN,
    );
    b.terminate(entry, RTerm::Return(None));
    let func = b.finish();
    let plan = plan_of(&func, &[]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let retargets = BTreeMap::from([(target, JsName::new("callee$h1$n1".to_owned()))]);

    let js = lower_retargeted(
        &interns,
        "caller$h2",
        &["callee$h1$n1"],
        &body,
        &func,
        &plan,
        straight(&func),
        &retargets,
    );
    assert!(js.contains("callee$h1$n1()"), "{js}");
    assert!(!js.contains("callee$h1()"), "{js}");
}

#[test]
fn cells_slots_and_direct_calls_match_the_14_pointers_entry() {
    let interns = world();
    // `let mut value = 1; bump(&mut value, 41); print_i32(value); let base = &arr[i]; ..`
    let mut b = FuncBuilder::new(item(1), signature(vec![], UNIT));
    let e = FuncBuilder::ENTRY;
    let cell = b.cell(I32, true, NO_ORIGIN);
    b.effect(e, OpKind::CellAlloc(cell), NO_ORIGIN);
    let one = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::CellStore { cell, v: one }, NO_ORIGIN);
    let slot = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Cell(cell),
            path: vec![],
            mutable: true,
        },
        REF_I32,
        NO_ORIGIN,
    );
    let forty_one = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 41, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let bump = interns.str(StrId(0)).map(|_| ()).is_some();
    let _ = bump;
    let mut interns = interns;
    let bump_name = interns.intern_str("x14_pointers$bump$h2b40639afb54a27c");
    let print_name = interns.intern_str("x14_pointers$mini_core$print_i32$hf678a2c85ecff833");
    b.effect(
        e,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(2),
                link_name: bump_name,
            },
            function: None,
            args: vec![slot, forty_one],
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        NO_ORIGIN,
    );
    let load = b.op(e, OpKind::CellLoad(cell), I32, NO_ORIGIN);
    b.effect(
        e,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(3),
                link_name: print_name,
            },
            function: None,
            args: vec![load],
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        NO_ORIGIN,
    );
    // `let base = &arr[_10]` with `arr = [10, 20, 30, 40]` and the bounds check.
    let arr = b.op(
        e,
        OpKind::Const(ConstTree::Aggregate(
            [10u128, 20, 30, 40]
                .iter()
                .map(|v| ConstTree::Scalar { bits: *v, bytes: 4 })
                .collect(),
        )),
        ARRAY,
        NO_ORIGIN,
    );
    let i = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let four = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 4, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let lt = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: i,
            r: four,
            ty: USIZE,
        },
        BOOL,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::Assert {
            cond: lt,
            expected: true,
            msg: RAssertMessage::BoundsCheck {
                length: four,
                index: i,
            },
        },
        NO_ORIGIN,
    );
    let base = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(arr),
            path: vec![Proj::Index(i)],
            mutable: false,
        },
        REF_I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(None));
    let func = b.finish();
    let mut plan = plan_of(&func, &[(i.0, "_10"), (base.0, "base")]);
    plan.cell_names.insert(cell, "value".to_owned());
    let body = body_of(BodyKind::Function, func.sig.clone());
    let deps = [
        "x14_pointers$bump$h2b40639afb54a27c",
        "x14_pointers$mini_core$panic_bounds_check$h25e30b2ab8a7433d",
        "x14_pointers$mini_core$print_i32$hf678a2c85ecff833",
    ];
    // Without an origin table there is no source fact for the panic location: the assert declines.
    let opts = Opts::default();
    let name = JsName::new("rust_entry".to_owned());
    let summary = summary(&deps);
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body: &body,
        origins: None,
        interns: &interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    let err = function::lower(&input, &func, &plan, straight(&func))
        .err()
        .expect("declines");
    assert!(err.to_string().contains("no source fact"), "{err}");

    // With the panic function absent from the deps the assert is the shim abort, and the rest
    // is the golden's shape.
    let deps = [
        "x14_pointers$bump$h2b40639afb54a27c",
        "x14_pointers$mini_core$print_i32$hf678a2c85ecff833",
    ];
    let js = lower(
        &interns,
        "rust_entry",
        &deps,
        &body,
        &func,
        &plan,
        straight(&func),
    );
    assert_eq!(
        js,
        "function rust_entry() {\n  let _10, base, value = [];\n  value[0] = 1;\n  x14_pointers$bump$h2b40639afb54a27c({ buf: value, off: 0 }, 41);\n  x14_pointers$mini_core$print_i32$hf678a2c85ecff833(value[0]);\n  _10 = 0;\n  if (!(_10 < 4)) {\n    __rt.js_abort(\"index out of bounds\");\n  }\n  base = { buf: [10, 20, 30, 40], off: _10 };\n}\n"
    );
}

#[test]
fn struct_aggregates_fields_and_copies() {
    let interns = world();
    // `fn f(p: &Point) -> Point { let q = *p; Point { x: q.y, y: q.x } }`
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_POINT], POINT));
    let p = b.param(REF_POINT, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let deref = b.op(e, OpKind::Deref(p), POINT, NO_ORIGIN);
    let q = b.op(e, OpKind::Copy(deref), POINT, NO_ORIGIN);
    let qy = b.op(
        e,
        OpKind::Field {
            base: q,
            variant: None,
            field: 1,
        },
        I32,
        NO_ORIGIN,
    );
    let qx = b.op(
        e,
        OpKind::Field {
            base: q,
            variant: None,
            field: 0,
        },
        I32,
        NO_ORIGIN,
    );
    let built = b.op(
        e,
        OpKind::Aggregate {
            ty: POINT,
            variant: 0,
            active_field: None,
            fields: vec![qy, qx],
        },
        POINT,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(built)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "p"), (q.0, "q")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(p) {\n  let q;\n  q = { x: p.x, y: p.y };\n  return { x: q.y, y: q.x };\n}\n"
    );
}

#[test]
fn set_field_and_write_through_a_reference() {
    let interns = world();
    // `fn f(p: &mut Point, arr: &mut [i32; 4], s: &mut [i32]) { p.x = 5; (*p) = Point{..}; arr[1] =
    // 2; s[0] = 3 }`
    let mut b = FuncBuilder::new(
        item(1),
        signature(vec![REF_POINT, REF_ARRAY, REF_SLICE], UNIT),
    );
    let p = b.param(REF_POINT, NO_ORIGIN);
    let arr = b.param(REF_ARRAY, NO_ORIGIN);
    let s = b.param(REF_SLICE, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let five = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let view = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(p),
            path: vec![Proj::Deref, Proj::Field { field: 0, ty: I32 }],
            mutable: true,
        },
        REF_I32,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::StoreThrough { ptr: view, v: five }, NO_ORIGIN);
    let one = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let two = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 2, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let fresh = b.op(
        e,
        OpKind::Aggregate {
            ty: POINT,
            variant: 0,
            active_field: None,
            fields: vec![one, two],
        },
        POINT,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::StoreThrough { ptr: p, v: fresh }, NO_ORIGIN);
    let idx = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let elem = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(arr),
            path: vec![Proj::Deref, Proj::Index(idx)],
            mutable: true,
        },
        REF_I32,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::StoreThrough { ptr: elem, v: two }, NO_ORIGIN);
    let zero = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let three = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 3, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let selem = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(s),
            path: vec![Proj::Deref, Proj::Index(zero)],
            mutable: true,
        },
        REF_I32,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::StoreThrough {
            ptr: selem,
            v: three,
        },
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(None));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "p"), (1, "arr"), (2, "s")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(p, arr, s) {\n  p.x = 5;\n  __rt.overwrite(p, { x: 1, y: 2 });\n  arr[1] = 2;\n  s.buf[s.off] = 3;\n}\n"
    );
}

#[test]
fn slices_from_arrays_and_subslices() {
    let interns = world();
    // `fn f(a: &[i32; 4]) -> &[i32] { &a[1..] }` and the unsize `a as &[i32]`.
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_ARRAY], REF_SLICE));
    let a = b.param(REF_ARRAY, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let whole = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::PointerCoercion,
            v: a,
            from: REF_ARRAY,
            to: REF_SLICE,
        },
        REF_SLICE,
        NO_ORIGIN,
    );
    let tail = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(whole),
            path: vec![
                Proj::Deref,
                Proj::Subslice {
                    from: 1,
                    to: 0,
                    from_end: true,
                },
            ],
            mutable: false,
        },
        REF_SLICE,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(tail)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "a"), (whole.0, "w")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(a) {\n  let w;\n  w = { buf: a, off: 0, len: 4 };\n  return { buf: w.buf, off: w.off + 1, len: w.len - 1 };\n}\n"
    );
}

#[test]
fn constants_spell_enums_strings_slices_and_hoisted_names() {
    let interns = world();
    let mut interns = interns;
    let hello = interns.intern_str("hi");
    let long = interns.intern_str("a string constant longer than thirty-two bytes");
    let mut b = FuncBuilder::new(item(1), signature(vec![], OPTION));
    let e = FuncBuilder::ENTRY;
    let some7 = b.op(
        e,
        OpKind::Const(ConstTree::Variant {
            variant: 1,
            fields: vec![ConstTree::Scalar { bits: 7, bytes: 4 }],
        }),
        OPTION,
        NO_ORIGIN,
    );
    let text = b.op(e, OpKind::Const(ConstTree::Str(hello)), STR_REF, NO_ORIGIN);
    let hoisted = b.op(e, OpKind::Const(ConstTree::Str(long)), STR_REF, NO_ORIGIN);
    let slice = b.op(
        e,
        OpKind::Const(ConstTree::Slice(vec![
            ConstTree::Scalar { bits: 1, bytes: 4 },
            ConstTree::Scalar { bits: 2, bytes: 4 },
        ])),
        REF_SLICE,
        NO_ORIGIN,
    );
    let point_ref = b.op(
        e,
        OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Aggregate(vec![
            ConstTree::Scalar { bits: 1, bytes: 4 },
            ConstTree::Scalar { bits: 2, bytes: 4 },
        ])))),
        REF_POINT,
        NO_ORIGIN,
    );
    let int_ref = b.op(
        e,
        OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Scalar {
            bits: 9,
            bytes: 4,
        }))),
        REF_I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(some7)));
    let func = b.finish();
    let plan = plan_of(
        &func,
        &[
            (text.0, "t"),
            (hoisted.0, "h"),
            (slice.0, "s"),
            (point_ref.0, "p"),
            (int_ref.0, "i"),
        ],
    );
    let body = body_of(BodyKind::Function, func.sig.clone());
    let key = super::constant::hoisted_name("s", "a string constant longer than thirty-two bytes");
    let js = lower(
        &interns,
        "f",
        &[key.as_str()],
        &body,
        &func,
        &plan,
        straight(&func),
    );
    assert_eq!(
        js,
        format!(
            "function f() {{\n  let t, h, s, p, i;\n  t = \"hi\";\n  h = {key};\n  s = {{ buf: [1, 2], off: 0, len: 2 }};\n  p = {{ x: 1, y: 2 }};\n  i = {{ buf: [9], off: 0 }};\n  return {{ TAG: \"Some\", _0: 7 }};\n}}\n"
        )
    );
    // Without the hoisted constant among the item's references the literal is inline.
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert!(
        js.contains("h = \"a string constant longer than thirty-two bytes\";"),
        "{js}"
    );
}

#[test]
fn niche_scalar_constants_name_the_niche_variant() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![], OPTION_REF));
    let e = FuncBuilder::ENTRY;
    let none = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        OPTION_REF,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(none)));
    let func = b.finish();
    let plan = plan_of(&func, &[]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(js, "function f() {\n  return { TAG: \"None\" };\n}\n");
}

#[test]
fn asserts_call_the_panic_lang_item_with_a_location() {
    let mut interns = world();
    let file = interns.intern_str("examples/emit/14_pointers.rs");
    let fact = interns.intern_source_fact(rcgjs_object::jir::origin::SourceFact {
        file,
        line: 68,
        col: 16,
    });
    let origins = rcgjs_object::jir::origin::OriginTable {
        origins: vec![rcgjs_object::jir::origin::Origin {
            kind: rcgjs_object::jir::origin::OriginKind::Terminator,
            source: Some(fact),
            local_name: None,
            inline_stack: Vec::new(),
        }],
        statements: Vec::new(),
        terminators: Vec::new(),
        locals: Vec::new(),
        item_path: interns.intern_str("rust_entry"),
        root: None,
    };
    let origin = rcgjs_object::jir::origin::OriginId(0);
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![USIZE], I32));
    let i = b.param(USIZE, NO_ORIGIN);
    let four = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 4, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let lt = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: i,
            r: four,
            ty: USIZE,
        },
        BOOL,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::Assert {
            cond: lt,
            expected: true,
            msg: RAssertMessage::BoundsCheck {
                length: four,
                index: i,
            },
        },
        origin,
    );
    let arr = b.op(
        e,
        OpKind::Const(ConstTree::Aggregate(
            [10u128, 20, 30, 40]
                .iter()
                .map(|v| ConstTree::Scalar { bits: *v, bytes: 4 })
                .collect(),
        )),
        ARRAY,
        NO_ORIGIN,
    );
    let view = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(arr),
            path: vec![Proj::Index(i)],
            mutable: false,
        },
        REF_I32,
        NO_ORIGIN,
    );
    let read = b.op(e, OpKind::Deref(view), I32, NO_ORIGIN);
    b.terminate(e, RTerm::Return(Some(read)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "_10"), (arr.0, "arr")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let opts = Opts::default();
    let name = JsName::new("f".to_owned());
    let summary = summary(&["x14_pointers$mini_core$panic_bounds_check$h25e30b2ab8a7433d"]);
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body: &body,
        origins: Some(&origins),
        interns: &interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    let lowered = function::lower(&input, &func, &plan, straight(&func)).expect("lowers");
    assert_eq!(
        jsast::stmt_to_string(&lowered.decl),
        "function f(_10) {\n  let arr;\n  if (!(_10 < 4)) {\n    x14_pointers$mini_core$panic_bounds_check$h25e30b2ab8a7433d(_10, 4, { file: \"examples/emit/14_pointers.rs\", line: 68, column: 17 });\n  }\n  arr = [10, 20, 30, 40];\n  return arr[_10];\n}\n"
    );
    let refs: BTreeSet<String> = lowered.refs.iter().map(|r| r.as_str().to_owned()).collect();
    assert!(refs.contains("x14_pointers$mini_core$panic_bounds_check$h25e30b2ab8a7433d"));
    assert!(!refs.contains("arr"));
}

#[test]
fn statics_and_vtables() {
    let mut interns = world();
    let opts = Opts::default();
    let name = JsName::new("S".to_owned());
    let summary = summary(&[]);
    let body = body_of(
        BodyKind::Static {
            init: ConstTree::Scalar { bits: 5, bytes: 4 },
            mutable: true,
        },
        signature(vec![], I32),
    );
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body: &body,
        origins: None,
        interns: &interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    let lowered = super::super::lower_item(&input).expect("static lowers");
    assert_eq!(jsast::stmt_to_string(&lowered.decl), "let S = [5];\n");

    let drop_name = interns.intern_str("drop$h0000000000000001");
    let method = interns.intern_str("method$h0000000000000002");
    let super_vtable = interns.intern_str("vtable$h0000000000000003");
    let body = body_of(
        BodyKind::Vtable {
            concrete: POINT,
            trait_ref: Some(def(40)),
            entries: vec![
                VtableEntry::Drop(Some(DirectItemRef {
                    item: item(9),
                    link_name: drop_name,
                })),
                VtableEntry::Size(8),
                VtableEntry::Align(4),
                VtableEntry::Method(DirectItemRef {
                    item: item(10),
                    link_name: method,
                }),
                VtableEntry::Vacant,
                VtableEntry::TraitVPtr(super_vtable),
            ],
        },
        signature(vec![], UNIT),
    );
    let name = JsName::new("V".to_owned());
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body: &body,
        origins: None,
        interns: &interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    let lowered = super::super::lower_item(&input).expect("vtable lowers");
    assert_eq!(
        jsast::stmt_to_string(&lowered.decl),
        "const V = [drop$h0000000000000001, 8, 4, method$h0000000000000002, null, vtable$h0000000000000003];\n"
    );
    let refs: BTreeSet<String> = lowered.refs.iter().map(|r| r.as_str().to_owned()).collect();
    assert_eq!(refs.len(), 3);
}

#[test]
fn a_dyn_drop_calls_vtable_slot_zero_when_it_is_not_null() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_POINT], UNIT));
    let p = b.param(REF_POINT, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    b.effect(
        e,
        OpKind::Drop {
            root: Root::Value(p),
            path: vec![Proj::Deref],
            glue: rcgjs_object::jir::semantic::DropGlue::Dynamic,
        },
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(None));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "p")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let opts = Opts::default();
    let name = JsName::new("f".to_owned());
    let summary = summary(&[]);
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body: &body,
        origins: None,
        interns: &interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    let lowered = function::lower(&input, &func, &plan, straight(&func)).expect("lowers");
    assert_eq!(
        jsast::stmt_to_string(&lowered.decl),
        "function f(p) {\n  if (p.meta[0] !== null) {\n    p.meta[0](p.ptr);\n  }\n}\n"
    );
}

#[test]
fn unsupported_ops_decline_with_a_class() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let p = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    // A representation-phase op has no baseline lowering at all.
    let v = b.op(e, OpKind::Materialize(p), I32, NO_ORIGIN);
    b.terminate(e, RTerm::Return(Some(v)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "p"), (1, "v")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let opts = Opts::default();
    let name = JsName::new("f".to_owned());
    let summary = summary(&[]);
    let input = LowerInput {
        name: &name,
        summary: &summary,
        body: &body,
        origins: None,
        interns: &interns,
        table: &interns.types,
        opts: &opts,
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    let err = function::lower(&input, &func, &plan, straight(&func))
        .err()
        .expect("declines");
    assert_eq!(err.category(), "lower");
    assert!(err.to_string().contains("Phase 3"), "{err}");
}

/// Keeps the unused imports honest.
#[allow(dead_code)]
fn _unused(_: BTreeMap<u32, u32>) {}

// -------------------------------------------------------------------------------------------
// W8: calls, drops, intrinsics, transmute
// -------------------------------------------------------------------------------------------

/// One source fact, and the origin table that names it on the entry block's one terminator.
fn one_fact(
    interns: &mut Interns,
    path: &str,
    line: u32,
    col: u32,
) -> rcgjs_object::jir::origin::SourceFactId {
    let file = interns.intern_str(path);
    interns.intern_source_fact(rcgjs_object::jir::origin::SourceFact { file, line, col })
}

/// Lowers a function with an item index and a vtable index behind it.
#[allow(clippy::too_many_arguments)]
fn lower_with(
    interns: &Interns,
    name: &str,
    body: &Body,
    func: &Func,
    plan: &Plan,
    region: Region<BlockPayload, RScrut>,
    items: &super::host::ItemIndex,
    vtables: &super::unsize::VtableIndex,
) -> String {
    let opts = Opts::default();
    let js_name = JsName::new(name.to_owned());
    let summary = summary(&[]);
    let input = LowerInput {
        name: &js_name,
        summary: &summary,
        body,
        origins: None,
        interns,
        table: &interns.types,
        opts: &opts,
        items,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
    };
    match function::lower(&input, func, plan, region) {
        Ok(item) => jsast::stmt_to_string(&item.decl),
        Err(decline) => panic!("declined: {decline}"),
    }
}

#[test]
fn a_call_returning_a_slot_is_materialized_before_dereference() {
    let mut interns = world();
    let link = interns.intern_str("next_value_pointer");
    let entry = FuncBuilder::ENTRY;
    let mut builder = FuncBuilder::new(item(1), signature(Vec::new(), I32));
    let pointer = builder.op(
        entry,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(2),
                link_name: link,
            },
            function: None,
            args: Vec::new(),
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        RAW_I32,
        NO_ORIGIN,
    );
    let value = builder.op(entry, OpKind::Deref(pointer), I32, NO_ORIGIN);
    builder.terminate(entry, RTerm::Return(Some(value)));
    let func = builder.finish();
    let opts = Opts::default();
    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &opts);

    assert!(!plan.inline.contains(&pointer));
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(
        &interns,
        "f",
        &["next_value_pointer"],
        &body,
        &func,
        &plan,
        straight(&func),
    );
    assert_eq!(
        js,
        "function f() {\n  let $v0;\n  $v0 = next_value_pointer();\n  return $v0.buf[$v0.off];\n}\n"
    );
}

#[test]
fn a_track_caller_call_passes_the_location_of_its_call_site() {
    let mut interns = world();
    let fact = one_fact(&mut interns, "examples/tests/05_recursion.rs", 12, 8);
    let link = interns.intern_str("g$h0000000000000002");
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let x = b.param(I32, NO_ORIGIN);
    let call = b.op(
        e,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(2),
                link_name: link,
            },
            function: None,
            args: vec![x],
            track_caller: Some(fact),
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(call)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "x")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(x) {\n  return g$h0000000000000002(x, { file: \"examples/tests/05_recursion.rs\", \
         line: 12, column: 9 });\n}\n"
    );
}

#[test]
fn a_call_inside_a_track_caller_body_forwards_its_own_location() {
    let mut interns = world();
    let link = interns.intern_str("g$h0000000000000002");
    let e = FuncBuilder::ENTRY;
    let mut sig = signature(vec![I32], I32);
    sig.abi.caller_location = true;
    sig.abi.parameters = 2;
    let mut b = FuncBuilder::new(item(1), sig);
    let x = b.param(I32, NO_ORIGIN);
    let call = b.op(
        e,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(2),
                link_name: link,
            },
            function: None,
            args: vec![x],
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(call)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "x")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    // The callee's own ABI is what says it takes the location: one argument plus `$loc`.
    let mut items = super::host::ItemIndex::new();
    items.insert(
        item(2),
        super::host::ItemEntry {
            abi_known: true,
            name: "g$h0000000000000002".to_owned(),
            params: 2,
            result: Some(I32),
            caller_location: true,
            used_params: BTreeSet::new(),
            standard: None,
            keyed_purity: None,
            keyed_read_params: None,
            keyed_may_unwind: None,
            keyed_row_capture: None,
            keyed_proxy_safe_params: None,
        },
    );
    let js = lower_with(
        &interns,
        "f",
        &body,
        &func,
        &plan,
        straight(&func),
        &items,
        &super::unsize::EMPTY_VTABLES,
    );
    assert_eq!(
        js,
        "function f(x) {\n  return g$h0000000000000002(x, $loc);\n}\n"
    );
    let mut zero = func.clone();
    for operation in zero.blocks.iter_mut().flat_map(|block| &mut block.ops) {
        if let OpKind::Call { args, .. } = &mut operation.kind {
            args.clear();
        }
    }
    let entry = items.get_mut(&item(2)).unwrap();
    entry.params = 0;
    entry.caller_location = false;
    let plan = plan_of(&zero, &[(0, "x")]);
    let js = lower_with(
        &interns,
        "f",
        &body,
        &zero,
        &plan,
        straight(&zero),
        &items,
        &super::unsize::EMPTY_VTABLES,
    );
    assert_eq!(js, "function f(x) {\n  return g$h0000000000000002();\n}\n");
}

#[test]
fn a_static_keyed_list_lowers_after_its_dedicated_contract_is_proven() {
    const COLLECTION_CLOSURE: TypeId = TypeId(40);
    const KEY_CLOSURE: TypeId = TypeId(41);
    const ROW_CLOSURE: TypeId = TypeId(42);

    let mut interns = world();
    for (ty, definition) in [
        (COLLECTION_CLOSURE, def(40)),
        (KEY_CLOSURE, def(41)),
        (ROW_CLOSURE, def(42)),
    ] {
        interns.intern_type(descriptor(
            ty,
            TypeKind::Closure {
                def: definition,
                upvars: Vec::new(),
                fn_pointer_shim: None,
            },
            0,
        ));
    }
    let marker_name = interns.intern_str("keyed_list_v1$h90");
    let collection_name = interns.intern_str("collection$h40");
    let key_name = interns.intern_str("key$h41");
    let row_name = interns.intern_str("row$h42");
    let callable = |ty, item, link_name| KeyedListCallable {
        ty,
        body: Some(DirectItemRef { item, link_name }),
        cleanup: DropGlue::None,
        source: None,
    };
    let payload = KeyedListPayload {
        format: KEYED_LIST_FORMAT_V1,
        site: None,
        def: def(90),
        collection_ty: ARRAY,
        item_ty: I32,
        key_ty: I32,
        row_ty: POINT,
        collection: callable(COLLECTION_CLOSURE, item(40), collection_name),
        key: callable(KEY_CLOSURE, item(41), key_name),
        row: callable(ROW_CLOSURE, item(42), row_name),
    };

    let entry = FuncBuilder::ENTRY;
    let mut builder = FuncBuilder::new(
        item(1),
        signature(vec![COLLECTION_CLOSURE, KEY_CLOSURE, ROW_CLOSURE], POINT),
    );
    let collection = builder.param(COLLECTION_CLOSURE, NO_ORIGIN);
    let key = builder.param(KEY_CLOSURE, NO_ORIGIN);
    let row = builder.param(ROW_CLOSURE, NO_ORIGIN);
    let result = builder.op(
        entry,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(90),
                link_name: marker_name,
            },
            function: None,
            args: vec![collection, key, row],
            track_caller: None,
            semantic: CallSemantic::ViewAbi,
            host: Some(HostPayload::KeyedList(Box::new(payload))),
        },
        POINT,
        NO_ORIGIN,
    );
    builder.terminate(entry, RTerm::Return(Some(result)));
    let func = builder.finish();
    let plan = plan_of(&func, &[(0, "collection"), (1, "key"), (2, "row")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let opts = Opts::default();
    let item_summary = summary(&[]);
    let mut items = super::host::ItemIndex::new();
    items.insert(
        item(40),
        super::host::ItemEntry {
            abi_known: true,
            name: "collection$h40".to_owned(),
            params: 1,
            result: Some(ARRAY),
            caller_location: false,
            used_params: BTreeSet::new(),
            standard: None,
            keyed_purity: Some(super::host::KeyedPurity::Const),
            keyed_read_params: Some(BTreeSet::new()),
            keyed_may_unwind: Some(false),
            keyed_row_capture: None,
            keyed_proxy_safe_params: Some(BTreeSet::new()),
        },
    );
    items.insert(
        item(41),
        super::host::ItemEntry {
            abi_known: true,
            name: "key$h41".to_owned(),
            params: 2,
            result: Some(I32),
            caller_location: false,
            used_params: BTreeSet::new(),
            standard: None,
            keyed_purity: Some(super::host::KeyedPurity::Const),
            keyed_read_params: Some(BTreeSet::new()),
            keyed_may_unwind: Some(false),
            keyed_row_capture: None,
            keyed_proxy_safe_params: Some(BTreeSet::new()),
        },
    );
    items.insert(
        item(42),
        super::host::ItemEntry {
            abi_known: true,
            name: "row$h42".to_owned(),
            params: 3,
            result: Some(POINT),
            caller_location: false,
            used_params: BTreeSet::new(),
            standard: None,
            keyed_purity: Some(super::host::KeyedPurity::Const),
            keyed_read_params: Some(BTreeSet::new()),
            keyed_may_unwind: Some(false),
            keyed_row_capture: Some(super::host::KeyedRowCapture::Static),
            keyed_proxy_safe_params: Some(BTreeSet::new()),
        },
    );
    let lower = |items: &super::host::ItemIndex| {
        let input = LowerInput {
            name: &JsName::new("f".to_owned()),
            summary: &item_summary,
            body: &body,
            origins: None,
            interns: &interns,
            table: &interns.types,
            opts: &opts,
            items,
            drop_glue: super::host::EMPTY_DROP_GLUE,
            vtables: &super::unsize::EMPTY_VTABLES,
            spills: &[],
            abi: None,
            physical: None,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
        };
        function::lower(&input, &func, &plan, straight(&func))
            .map(|lowered| jsast::stmt_to_string(&lowered.decl))
            .map_err(|decline| decline.to_string())
    };
    let js = lower(&items).expect("a creation-only keyed row lowers");
    assert!(
        js.contains("__rt.keyed_dom_view("),
        "the executable marker uses the transactional DOM adapter: {js}"
    );
    for import in [
        "_$batch",
        "_$createRoot",
        "_$createSignal",
        "_$effect",
        "_$onCleanup",
        "_$untrack",
    ] {
        assert!(
            js.contains(import),
            "the keyed hooks include {import}: {js}"
        );
    }

    items.get_mut(&item(42)).unwrap().keyed_row_capture = Some(super::host::KeyedRowCapture::Live);
    assert!(
        lower(&items)
            .expect_err("a live row capture declines")
            .contains("keyed.row-live-item-not-ready"),
        "the first live lane requires an immutable item reference"
    );

    items.get_mut(&item(42)).unwrap().keyed_row_capture =
        Some(super::host::KeyedRowCapture::Static);
    items.get_mut(&item(41)).unwrap().keyed_purity = Some(super::host::KeyedPurity::Read);
    items.get_mut(&item(41)).unwrap().keyed_read_params = Some(BTreeSet::from([1]));
    lower(&items).expect("a key may read through its item argument");

    items.get_mut(&item(41)).unwrap().keyed_read_params = Some(BTreeSet::from([0]));
    assert!(
        lower(&items)
            .expect_err("a capture-dependent key declines")
            .contains("keyed.key-extractor-reads-state"),
        "a read through the closure environment is not a deterministic key proof"
    );

    items.get_mut(&item(41)).unwrap().keyed_read_params = Some(BTreeSet::from([1]));
    items.get_mut(&item(41)).unwrap().keyed_may_unwind = Some(true);
    assert!(
        lower(&items)
            .expect_err("a fallible key declines")
            .contains("keyed.key-extractor-fallible"),
        "key extraction must finish before any row transaction starts"
    );
}

#[test]
fn a_virtual_call_dispatches_through_the_receivers_vtable_slot() {
    let interns = world();
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![SHAPE], I32));
    let receiver = b.param(SHAPE, NO_ORIGIN);
    let call = b.op(
        e,
        OpKind::Call {
            callee: Callee::Virtual {
                slot: 3,
                receiver: SHAPE,
            },
            function: None,
            args: vec![receiver],
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(call)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "s")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(js, "function f(s) {\n  return s.meta[3](s.ptr);\n}\n");
}

#[test]
fn erased_pointer_casts_restore_the_target_units() {
    let mut interns = world();
    let erased = TypeId(500);
    interns.types.insert(descriptor(
        erased,
        TypeKind::RawPointer {
            pointee: UNIT,
            mutable: false,
        },
        4,
    ));
    for (target, size) in [(RAW_I32, 4), (RAW_POINT, 8)] {
        let mut b = FuncBuilder::new(item(1), signature(vec![erased], target));
        let pointer = b.param(erased, NO_ORIGIN);
        let restored = b.op(
            FuncBuilder::ENTRY,
            OpKind::Cast {
                kind: CastKind::PtrToPtr,
                v: pointer,
                from: erased,
                to: target,
            },
            target,
            NO_ORIGIN,
        );
        b.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(restored)));
        let func = b.finish();
        let plan = plan_of(&func, &[(0, "pointer")]);
        let body = body_of(BodyKind::Function, func.sig.clone());
        let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
        assert_eq!(
            js,
            format!("function f(pointer) {{\n  return __rt.thin(pointer, {size});\n}}\n")
        );
    }
}

#[test]
fn an_unsized_virtual_receiver_keeps_its_fat_pointer_identity() {
    let interns = world();
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_DYNAMIC], I32));
    let receiver = b.param(REF_DYNAMIC, NO_ORIGIN);
    let pointee = b.op(e, OpKind::Deref(receiver), DYNAMIC, NO_ORIGIN);
    let call = b.op(
        e,
        OpKind::Call {
            callee: Callee::Virtual {
                slot: 3,
                receiver: DYNAMIC,
            },
            function: None,
            args: vec![pointee],
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(call)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "receiver")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(receiver) {\n  return receiver.meta[3](__rt.unref(receiver.ptr));\n}\n"
    );
}

#[test]
fn a_direct_drop_calls_the_selected_glue_with_a_reference_to_the_place() {
    let mut interns = world();
    let link = interns.intern_str("drop_point$h0000000000000003");
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![POINT], UNIT));
    let p = b.param(POINT, NO_ORIGIN);
    b.effect(
        e,
        OpKind::Drop {
            root: Root::Value(p),
            path: Vec::new(),
            glue: rcgjs_object::jir::semantic::DropGlue::Direct {
                item: item(3),
                link_name: link,
            },
        },
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(None));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "p")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(p) {\n  drop_point$h0000000000000003(p);\n}\n"
    );
    let retargets = BTreeMap::from([(
        item(3),
        JsName::new("drop_point$h0000000000000003$n1".to_owned()),
    )]);
    let js = lower_retargeted(
        &interns,
        "f",
        &[],
        &body,
        &func,
        &plan,
        straight(&func),
        &retargets,
    );
    assert_eq!(
        js,
        "function f(p) {\n  drop_point$h0000000000000003$n1(p);\n}\n"
    );
}

#[test]
fn only_a_decoded_static_template_operand_can_disappear_into_its_marker() {
    for case in 0..8 {
        let mut interns = world();
        let section = interns.intern_str(if case == 4 {
            "rcgjs.host.unknown"
        } else {
            "rcgjs.tc.template"
        });
        let link_name = interns.intern_str("template_marker");
        let source_name = interns.intern_str("template_source");
        let mut builder = FuncBuilder::new(item(1), signature(Vec::new(), I32));
        let entry = FuncBuilder::ENTRY;
        let source = if case == 2 {
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name: source_name,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            }
        } else {
            OpKind::Const(ConstTree::Global {
                target: rcgjs_object::jir::semantic::GlobalReference::Static {
                    definition: def(2),
                    link_name: source_name,
                },
                offset: if case == 1 { 4 } else { 0 },
            })
        };
        let argument = builder.op(entry, source, REF_POINT, NO_ORIGIN);
        let result = builder.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(3),
                    link_name,
                },
                function: None,
                args: if case == 5 {
                    vec![argument, argument]
                } else {
                    vec![argument]
                },
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: Some(HostPayload::Marker {
                    section,
                    def: def(3),
                    template: (case != 3).then_some(ConstTree::Unit),
                    helpers: Vec::new(),
                }),
            },
            I32,
            NO_ORIGIN,
        );
        if case >= 6 {
            builder.op(
                entry,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(3),
                        link_name,
                    },
                    function: None,
                    args: vec![argument],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: (case == 6).then_some(HostPayload::Marker {
                        section,
                        def: def(3),
                        template: Some(ConstTree::Unit),
                        helpers: Vec::new(),
                    }),
                },
                I32,
                NO_ORIGIN,
            );
        }
        builder.terminate(entry, RTerm::Return(Some(result)));
        let func = builder.finish();
        func.verify(&interns.types).unwrap();
        let plan =
            super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
        assert_eq!(
            plan.inline.contains(&argument),
            case == 0 || case == 6,
            "case {case}"
        );
    }
}

#[test]
fn branching_intrinsics_capture_effectful_arguments_before_emission() {
    for intrinsic in [
        "minimum_number_nsz_f32",
        "maximum_number_nsz_f32",
        "copysignf32",
    ] {
        let mut interns = world();
        let name = interns.intern_str(intrinsic);
        let mut builder = FuncBuilder::new(item(1), signature(Vec::new(), F32));
        let entry = FuncBuilder::ENTRY;
        let arguments = ["left_read", "right_read"].map(|name| {
            let link_name = interns.intern_str(name);
            builder.op(
                entry,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: item(2),
                        link_name,
                    },
                    function: None,
                    args: Vec::new(),
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                F32,
                NO_ORIGIN,
            )
        });
        let result = builder.op(
            entry,
            OpKind::Intrinsic {
                name,
                types: Vec::new(),
                fallback: None,
                args: arguments.to_vec(),
            },
            F32,
            NO_ORIGIN,
        );
        builder.terminate(entry, RTerm::Return(Some(result)));
        let func = builder.finish();
        let opts = Opts::default();
        let fused = super::super::fuse::plan(&func, None, &interns.types, &interns, &opts);
        assert!(arguments.iter().all(|arg| !fused.inline.contains(arg)));
        // Also exercise emission independently of fusion's guard. Both consumers use the
        // same evaluation contract, so a hand-built plan cannot duplicate a host expression.
        for plan in [fused, plan_of(&func, &[])] {
            let body = body_of(BodyKind::Function, func.sig.clone());
            let js = lower(
                &interns,
                "f",
                &["left_read", "right_read"],
                &body,
                &func,
                &plan,
                straight(&func),
            );
            assert_eq!(js.matches("left_read()").count(), 1, "{intrinsic}: {js}");
            assert_eq!(js.matches("right_read()").count(), 1, "{intrinsic}: {js}");
            assert!(
                js.find("left_read()").unwrap() < js.find("right_read()").unwrap(),
                "{js}"
            );
            assert!(
                js.find("right_read()").unwrap() < js.find("return ").unwrap(),
                "{js}"
            );
        }
    }
}

#[test]
fn a_synthetic_zero_sized_place_preserves_ordered_operand_effects() {
    for raw in [false, true] {
        let mut interns = world();
        interns.types.replace(descriptor(
            ARRAY,
            TypeKind::Array {
                element: UNIT,
                length: 4,
            },
            0,
        ));
        let mut builder = FuncBuilder::new(item(1), signature(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let [base, index, value] =
            [("base", REF_ARRAY), ("index", USIZE), ("value", UNIT)].map(|(name, ty)| {
                let link_name = interns.intern_str(name);
                builder.op(
                    entry,
                    OpKind::Call {
                        callee: Callee::Direct {
                            item: item(2),
                            link_name,
                        },
                        function: None,
                        args: Vec::new(),
                        track_caller: None,
                        semantic: CallSemantic::Ordinary,
                        host: None,
                    },
                    ty,
                    NO_ORIGIN,
                )
            });
        let root = Root::Value(base);
        let path = vec![Proj::Deref, Proj::Index(index)];
        let view = builder.op(
            entry,
            if raw {
                OpKind::RawView {
                    root,
                    path,
                    mutable: true,
                }
            } else {
                OpKind::BorrowView {
                    root,
                    path,
                    mutable: true,
                }
            },
            // SSA uses the pointee type when no actual reference descriptor exists.
            UNIT,
            NO_ORIGIN,
        );
        builder.effect(
            entry,
            OpKind::StoreThrough {
                ptr: view,
                v: value,
            },
            NO_ORIGIN,
        );
        builder.terminate(entry, RTerm::Return(None));
        let func = builder.finish();
        func.verify(&interns.types).unwrap();
        let plan =
            super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
        assert!(
            plan.inline.contains(&view),
            "a synthetic place must reach its store"
        );
        assert!(
            !plan.inline.contains(&value),
            "a zero-sized call must execute"
        );
        let body = body_of(BodyKind::Function, func.sig.clone());
        let js = lower(
            &interns,
            "f",
            &["base", "index", "value"],
            &body,
            &func,
            &plan,
            straight(&func),
        );
        for call in ["base()", "index()", "value()"] {
            assert_eq!(js.matches(call).count(), 1, "{js}");
        }
        assert!(
            js.find("base()").unwrap() < js.find("index()").unwrap(),
            "{js}"
        );
        assert!(
            js.find("index()").unwrap() < js.find("value()").unwrap(),
            "{js}"
        );
    }
}

#[test]
fn scalar_closure_constants_keep_capture_slots_and_signed_values() {
    let mut interns = world();
    let closure = TypeId(32);
    interns.intern_type(descriptor(
        closure,
        TypeKind::Closure {
            def: def(32),
            upvars: vec![UNIT, I32, UNIT],
            fn_pointer_shim: None,
        },
        4,
    ));
    let mut builder = FuncBuilder::new(item(1), signature(Vec::new(), closure));
    let entry = FuncBuilder::ENTRY;
    let value = builder.op(
        entry,
        OpKind::Const(ConstTree::Scalar {
            bits: (-7i32 as u32).into(),
            bytes: 4,
        }),
        closure,
        NO_ORIGIN,
    );
    builder.terminate(entry, RTerm::Return(Some(value)));
    let func = builder.finish();
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(
        &interns,
        "f",
        &[],
        &body,
        &func,
        &plan_of(&func, &[]),
        straight(&func),
    );
    assert_eq!(
        js,
        "function f() {\n  return [undefined, -7, undefined];\n}\n"
    );
}

#[test]
fn fixed_host_arrays_require_private_unchanged_storage() {
    for (empty, observed) in [
        (false, None),
        (false, Some(0)),
        (false, Some(1)),
        (false, Some(2)),
        (true, None),
        (true, Some(1)),
        (true, Some(2)),
    ] {
        let mut interns = world();
        if empty {
            interns.types.replace(descriptor(
                ARRAY,
                TypeKind::Array {
                    element: I32,
                    length: 0,
                },
                0,
            ));
        }
        for reference in [REF_ARRAY, REF_SLICE] {
            let mut ty = interns.types.get(reference).unwrap().clone();
            if let TypeKind::Reference { mutable, .. } = &mut ty.kind {
                *mutable = false;
            }
            interns.types.replace(ty);
        }
        let mut element = interns.types.get(I32).unwrap().clone();
        element.meta.boundary = rcgjs_object::jir::types::PinnedBoundary::Host;
        interns.types.replace(element);
        let section = interns.intern_str("rcgjs.host.invoke");
        let link_name = interns.intern_str("invoke");
        let mut builder = FuncBuilder::new(item(1), signature(vec![I32], I32));
        let entry = FuncBuilder::ENTRY;
        let value = builder.param(I32, NO_ORIGIN);
        let array = builder.op(
            entry,
            OpKind::Aggregate {
                ty: ARRAY,
                variant: 0,
                active_field: None,
                fields: if empty { Vec::new() } else { vec![value; 4] },
            },
            ARRAY,
            NO_ORIGIN,
        );
        let borrow = builder.op(
            entry,
            if empty {
                OpKind::Const(ConstTree::Reference(Box::new(ConstTree::Aggregate(
                    Vec::new(),
                ))))
            } else {
                OpKind::BorrowView {
                    root: Root::Value(array),
                    path: Vec::new(),
                    mutable: false,
                }
            },
            REF_ARRAY,
            NO_ORIGIN,
        );
        let slice = builder.op(
            entry,
            OpKind::Cast {
                kind: CastKind::PointerCoercion,
                v: borrow,
                from: REF_ARRAY,
                to: REF_SLICE,
            },
            REF_SLICE,
            NO_ORIGIN,
        );
        if let Some(index) = observed {
            let (v, ty) = [(array, ARRAY), (borrow, REF_ARRAY), (slice, REF_SLICE)][index];
            builder.op(entry, OpKind::Copy(v), ty, NO_ORIGIN);
        }
        let call = builder.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name,
                },
                function: None,
                args: vec![value, value, slice],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: Some(HostPayload::Marker {
                    section,
                    def: def(2),
                    template: None,
                    helpers: Vec::new(),
                }),
            },
            I32,
            NO_ORIGIN,
        );
        builder.terminate(entry, RTerm::Return(Some(call)));
        let original = builder.finish();
        original.verify(&interns.types).unwrap();
        let mut func = original.clone();
        let stats = crate::opt::host_args::run(&mut func, &interns.types, &interns);
        if observed.is_some() {
            assert_eq!(stats.would_lower, 0);
            assert_eq!(stats.declines["host.argument-pack-escapes"], 1);
            assert_eq!(func, original);
        } else {
            assert_eq!(stats.would_lower, 1, "{stats:?}");
            func.verify(&interns.types).unwrap();
            assert!(matches!(
                func.blocks[0].ops.last().unwrap().kind,
                OpKind::Host { .. }
            ));
            let fused =
                super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
            assert!(!fused.inline.contains(&if empty { slice } else { array }));
            let body = body_of(BodyKind::Function, func.sig.clone());
            let js = lower(&interns, "f", &[], &body, &func, &fused, straight(&func));
            assert!(js.contains("Reflect.apply"), "{js}");
            assert!(!js.contains(".slice("), "{js}");
            if empty {
                assert!(js.contains("[]"), "{js}");
                assert_eq!(
                    crate::analysis::alloc_sites::AllocSites::of(&func, &interns).materialize,
                    1,
                );
            }
        }
    }
}

#[test]
fn aggregate_repeat_captures_its_producer_outside_the_callback() {
    for count in [0, 2] {
        let mut interns = world();
        let array_ty = TypeId(32);
        interns.types.insert(descriptor(
            array_ty,
            TypeKind::Array {
                element: POINT,
                length: count,
            },
            8 * count,
        ));
        let link_name = interns.intern_str("produce_point");
        let entry = FuncBuilder::ENTRY;
        let mut builder = FuncBuilder::new(item(1), signature(Vec::new(), array_ty));
        let argument = builder.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            POINT,
            NO_ORIGIN,
        );
        let result = builder.op(
            entry,
            OpKind::Repeat { v: argument, count },
            array_ty,
            NO_ORIGIN,
        );
        builder.terminate(entry, RTerm::Return(Some(result)));
        let func = builder.finish();
        let opts = Opts::default();
        let fused = super::super::fuse::plan(&func, None, &interns.types, &interns, &opts);
        assert!(!fused.inline.contains(&argument));
        let plans = if count == 0 {
            vec![fused]
        } else {
            vec![fused, plan_of(&func, &[])]
        };
        for plan in plans {
            let body = body_of(BodyKind::Function, func.sig.clone());
            let js = lower(
                &interns,
                "f",
                &["produce_point"],
                &body,
                &func,
                &plan,
                straight(&func),
            );
            assert_eq!(js.matches("produce_point()").count(), 1, "{js}");
            if count != 0 {
                assert!(
                    js.find("produce_point()").unwrap() < js.find("=>").unwrap(),
                    "{js}"
                );
            }
        }
    }
}

#[test]
fn discriminant_evaluates_its_producer_once_for_constant_and_branching_results() {
    for enum_ty in [POINT, SHAPE] {
        let mut interns = world();
        let link_name = interns.intern_str("produce");
        let entry = FuncBuilder::ENTRY;
        let mut builder = FuncBuilder::new(item(1), signature(Vec::new(), I32));
        let argument = builder.op(
            entry,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(2),
                    link_name,
                },
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            enum_ty,
            NO_ORIGIN,
        );
        let result = builder.op(entry, OpKind::Discriminant(argument), I32, NO_ORIGIN);
        builder.terminate(entry, RTerm::Return(Some(result)));
        let func = builder.finish();
        let opts = Opts::default();
        let fused = super::super::fuse::plan(&func, None, &interns.types, &interns, &opts);
        assert!(!fused.inline.contains(&argument));
        for plan in [fused, plan_of(&func, &[])] {
            let body = body_of(BodyKind::Function, func.sig.clone());
            let js = lower(
                &interns,
                "f",
                &["produce"],
                &body,
                &func,
                &plan,
                straight(&func),
            );
            assert_eq!(js.matches("produce()").count(), 1, "{js}");
        }
    }
}

#[test]
fn an_intrinsic_with_a_prelude_emits_its_statements_before_the_binding() {
    let mut interns = world();
    let name = interns.intern_str("saturating_add");
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![U8, U8], U8));
    let x = b.param(U8, NO_ORIGIN);
    let y = b.param(U8, NO_ORIGIN);
    let sum = b.op(
        e,
        OpKind::Intrinsic {
            name,
            types: vec![U8],
            fallback: None,
            args: vec![x, y],
        },
        U8,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(sum)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "x"), (1, "y"), (sum.0, "s")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        concat!(
            "function f(x, y) {\n  let s, $t0;\n  $t0 = x + y;\n",
            "  s = $t0 > 255 ? 255 : $t0 < 0 ? 0 : $t0;\n  return s;\n}\n"
        )
    );
}

#[test]
fn an_unknown_intrinsic_falls_back_to_the_body_the_collector_emitted() {
    let mut interns = world();
    let name = interns.intern_str("not_an_intrinsic_this_backend_knows");
    let link = interns.intern_str("fallback$h0000000000000004");
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let x = b.param(I32, NO_ORIGIN);
    let out = b.op(
        e,
        OpKind::Intrinsic {
            name,
            types: vec![I32],
            fallback: Some(rcgjs_object::jir::semantic::DirectItemRef {
                item: item(4),
                link_name: link,
            }),
            args: vec![x],
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(out)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "x")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(x) {\n  return fallback$h0000000000000004(x);\n}\n"
    );
}

#[test]
fn a_transmute_between_two_integers_of_one_width_remasks_the_value() {
    let interns = world();
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![U32], I32));
    let x = b.param(U32, NO_ORIGIN);
    let out = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::Transmute,
            v: x,
            from: U32,
            to: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(out)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "x")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(js, "function f(x) {\n  return x | 0;\n}\n");
}

/// `fn f(s: &mut [()]) { sink(&raw mut (*s)[index()]); sink(s as *mut ()); drop((*s)[0]) }`, and
/// the same over a zero-sized `[(); 4]` behind a reference: a zero-sized element has no storage,
/// so instead of a read of `s.buf[s.off + i]` (a `TypeError` for a slice unsized from an
/// `undefined` array) its raw pointer, the cast of the container to an element pointer
/// (`as_ptr`), and the reference its glue receives are all one address: the slice's data pointer
/// `s.buf`, or the array's own pointer `s`. The index is not spelled in the place, so the plan
/// keeps it at its definition, where its call still runs once; an `i32` element spells the same
/// index inline.
#[test]
fn a_zero_sized_element_is_a_place_with_no_storage() {
    const UNIT_SLICE: TypeId = TypeId(40);
    const REF_UNIT_SLICE: TypeId = TypeId(41);
    const RAW_UNIT: TypeId = TypeId(42);
    const UNIT_ARRAY: TypeId = TypeId(43);
    const REF_UNIT_ARRAY: TypeId = TypeId(44);
    let mut interns = world();
    let mut unit_slice = descriptor(UNIT_SLICE, TypeKind::Slice(UNIT), 0);
    unit_slice.layout.sized = false;
    unit_slice.layout.size = None;
    unit_slice.layout.zst = false;
    unit_slice.js.is_indirect = true;
    interns.intern_type(unit_slice);
    interns.intern_type(pointer(
        REF_UNIT_SLICE,
        UNIT_SLICE,
        PointerForm::FatSlice,
        false,
    ));
    interns.intern_type(pointer(RAW_UNIT, UNIT, PointerForm::Thin, true));
    interns.intern_type(descriptor(
        UNIT_ARRAY,
        TypeKind::Array {
            element: UNIT,
            length: 4,
        },
        0,
    ));
    interns.intern_type(pointer(
        REF_UNIT_ARRAY,
        UNIT_ARRAY,
        PointerForm::Thin,
        false,
    ));
    let sink = interns.intern_str("sink");
    let index_name = interns.intern_str("index");
    let glue = interns.intern_str("drop_unit");

    for (base_ty, element_ptr) in [
        (REF_UNIT_SLICE, RAW_UNIT),
        (REF_UNIT_ARRAY, RAW_UNIT),
        (REF_SLICE, RAW_I32),
    ] {
        let zero_sized = element_ptr == RAW_UNIT;
        let mut b = FuncBuilder::new(item(1), signature(vec![base_ty], UNIT));
        let s = b.param(base_ty, NO_ORIGIN);
        let e = FuncBuilder::ENTRY;
        let call = |callee, args| OpKind::Call {
            callee: Callee::Direct {
                item: item(2),
                link_name: callee,
            },
            function: None,
            args,
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        };
        let index = b.op(e, call(index_name, Vec::new()), USIZE, NO_ORIGIN);
        let element = b.op(
            e,
            OpKind::RawView {
                root: Root::Value(s),
                path: vec![Proj::Deref, Proj::Index(index)],
                mutable: true,
            },
            element_ptr,
            NO_ORIGIN,
        );
        b.effect(e, call(sink, vec![element]), NO_ORIGIN);
        if zero_sized {
            let as_ptr = b.op(
                e,
                OpKind::Cast {
                    kind: CastKind::PtrToPtr,
                    v: s,
                    from: base_ty,
                    to: RAW_UNIT,
                },
                RAW_UNIT,
                NO_ORIGIN,
            );
            b.effect(e, call(sink, vec![as_ptr]), NO_ORIGIN);
            b.effect(
                e,
                OpKind::Drop {
                    root: Root::Value(s),
                    path: vec![
                        Proj::Deref,
                        Proj::ConstantIndex {
                            offset: 0,
                            min_length: 1,
                            from_end: false,
                        },
                    ],
                    glue: DropGlue::Direct {
                        item: item(3),
                        link_name: glue,
                    },
                },
                NO_ORIGIN,
            );
        }
        b.terminate(e, RTerm::Return(None));
        let func = b.finish();
        func.verify(&interns.types).unwrap();
        let plan =
            super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
        assert_eq!(
            plan.inline.contains(&index),
            !zero_sized,
            "an index the place does not spell must not be inline"
        );
        let body = body_of(BodyKind::Function, func.sig.clone());
        let js = lower(
            &interns,
            "f",
            &["sink", "index", "drop_unit"],
            &body,
            &func,
            &plan,
            straight(&func),
        );
        assert_eq!(js.matches("index()").count(), 1, "{js}");
        if zero_sized {
            let address = if base_ty == REF_UNIT_SLICE {
                "$v0.buf"
            } else {
                "$v0"
            };
            assert_eq!(js.matches(&format!("sink({address});")).count(), 2, "{js}");
            assert!(js.contains(&format!("drop_unit({address});")), "{js}");
            assert!(!js.contains("["), "{js}");
        } else {
            assert!(js.contains(".off + index()"), "{js}");
        }
    }
}

mod invoke;
