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
            inline_never: false,
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
        bigints: None,
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

/// `(r, o) = a.op(b)` returned whole: `p = op(a, b)` is the pair itself.
fn escaping_overflow(op: BinaryOp, ty: TypeId, pair: TypeId) -> Func {
    let mut b = FuncBuilder::new(item(1), signature(vec![ty, ty], pair));
    let x = b.param(ty, NO_ORIGIN);
    let y = b.param(ty, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let p = b.op(e, OpKind::Binary { op, l: x, r: y, ty }, pair, NO_ORIGIN);
    b.terminate(e, RTerm::Return(Some(p)));
    b.finish()
}

/// `let (r, o) = a.op(b); (r, o)`: the pair `p` is only ever projected, and a fresh tuple of its
/// two fields is what leaves. Values: `a b p r o out`.
fn projected_overflow(op: BinaryOp, ty: TypeId, pair: TypeId) -> Func {
    let mut b = FuncBuilder::new(item(1), signature(vec![ty, ty], pair));
    let x = b.param(ty, NO_ORIGIN);
    let y = b.param(ty, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let p = b.op(e, OpKind::Binary { op, l: x, r: y, ty }, pair, NO_ORIGIN);
    let field = |b: &mut FuncBuilder, field, ty| {
        b.op(
            e,
            OpKind::Field {
                base: p,
                variant: None,
                field,
            },
            ty,
            NO_ORIGIN,
        )
    };
    let wrapped = field(&mut b, 0, ty);
    let overflowed = field(&mut b, 1, BOOL);
    let out = b.op(
        e,
        OpKind::Aggregate {
            ty: pair,
            variant: 0,
            active_field: None,
            fields: vec![wrapped, overflowed],
        },
        pair,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(out)));
    b.finish()
}

/// One integer width the overflow tests sweep, with the types its pairs need.
#[derive(Clone, Copy)]
struct Width {
    element: TypeId,
    /// `(T, bool)`.
    pair: TypeId,
    /// `((T, bool),)`, a product holding a pair in a field.
    nest: TypeId,
    /// `&(T, bool)` and `*mut (T, bool)`.
    reference: TypeId,
    raw: TypeId,
    signed: bool,
    bits: u32,
}

impl Width {
    fn label(self) -> String {
        format!("{}{}", if self.signed { 'i' } else { 'u' }, self.bits)
    }
}

/// The test world plus every integer width the overflow tests sweep: `u32`, `i8`, `u8`, `u16`,
/// `i32`, `i64`, `u64`, in that order.
fn overflow_world() -> (Interns, Vec<Width>) {
    const I8: TypeId = TypeId(60);
    const U16: TypeId = TypeId(61);
    const U64: TypeId = TypeId(62);
    let mut interns = world();
    interns.intern_type(int(I8, true, 8));
    interns.intern_type(int(U16, false, 16));
    interns.intern_type(int(U64, false, 64));
    let mut widths = Vec::new();
    for (index, (element, signed, bits)) in [
        (U32, false, 32),
        (I8, true, 8),
        (U8, false, 8),
        (U16, false, 16),
        (I32, true, 32),
        (I64, true, 64),
        (U64, false, 64),
    ]
    .into_iter()
    .enumerate()
    {
        let id = |slot: u128| TypeId(100 + 10 * index as u128 + slot);
        let pair = if element == U32 { OVERFLOW } else { id(0) };
        if element != U32 {
            let mut descriptor = descriptor(pair, TypeKind::Tuple(vec![element, BOOL]), 16);
            descriptor.js.is_indirect = true;
            descriptor.js.needs_clone = true;
            descriptor.js.fields = vec![
                field(FieldKey::Index(0), element),
                field(FieldKey::Index(1), BOOL),
            ];
            interns.intern_type(descriptor);
        }
        let mut nest = descriptor(id(1), TypeKind::Tuple(vec![pair]), 16);
        nest.js.is_indirect = true;
        nest.js.needs_clone = true;
        nest.js.fields = vec![field(FieldKey::Index(0), pair)];
        interns.intern_type(nest);
        interns.intern_type(pointer(id(2), pair, PointerForm::Thin, false));
        interns.intern_type(pointer(id(3), pair, PointerForm::Thin, true));
        widths.push(Width {
            element,
            pair,
            nest: id(1),
            reference: id(2),
            raw: id(3),
            signed,
            bits,
        });
    }
    (interns, widths)
}

/// A Node program that runs lowered `(a, b) -> (T, bool)` functions over every pair of a width's
/// boundary values and compares each answer with Rust's own `overflowing_*`.
struct OverflowHarness {
    source: String,
}

impl OverflowHarness {
    fn new() -> Self {
        let mut source = String::from(
            "'use strict';\nconst assert = require('node:assert/strict');\nconst cases = [];\n\
             function keep(p) {\n  assert.ok(Array.isArray(p) && p.length === 2, \
             `keep got ${p}`);\n  return p;\n}\n",
        );
        source.push_str(&super::i64_pair::helper_preamble_js());
        Self { source }
    }

    /// Adds the function `js` (declaring `name`) and its cases for `op` at `width`.
    fn add(&mut self, name: &str, js: &str, op: BinaryOp, width: Width) {
        let Width { signed, bits, .. } = width;
        let (min, max): (i128, i128) = if signed {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        } else {
            (0, (1i128 << bits) - 1)
        };
        let half = 1i128 << (bits / 2);
        let edges = [min, min + 1, max - 1, max, -half, half - 1, half];
        let mut values: Vec<i128> = edges.into_iter().chain(-2..=2).collect();
        values.retain(|value| (min..=max).contains(value));
        let wrap = |value: i128| {
            let modulus = 1i128 << bits;
            let low = value.rem_euclid(modulus);
            if signed && low > max {
                low - modulus
            } else {
                low
            }
        };
        let literal = |value: i128| {
            if bits > 32 {
                format!("{value}n")
            } else {
                value.to_string()
            }
        };
        self.source.push_str(js);
        for &a in &values {
            for &b in &values {
                // A 64 bit product can leave `i128` too; it has overflowed either way, and
                // wrapping modulo 2^128 keeps the low bits.
                let (wrapped, overflowed) = match op {
                    BinaryOp::AddWithOverflow => (wrap(a + b), wrap(a + b) != a + b),
                    BinaryOp::SubWithOverflow => (wrap(a - b), wrap(a - b) != a - b),
                    _ => match a.checked_mul(b) {
                        Some(exact) => (wrap(exact), wrap(exact) != exact),
                        None => (wrap(a.wrapping_mul(b)), true),
                    },
                };
                self.source.push_str(&format!(
                    "cases.push(['{name}', {name}, {}, {}, {}, {overflowed}]);\n",
                    literal(a),
                    literal(b),
                    literal(wrapped),
                ));
            }
        }
    }

    /// Lowers `func` as `name` with `names` for its values and adds it.
    fn lower(
        &mut self,
        interns: &Interns,
        name: &str,
        func: &Func,
        names: &[(u32, &str)],
        op: BinaryOp,
        width: Width,
    ) -> String {
        let mut plan = plan_of(func, names);
        for (index, _) in func.cells.iter().enumerate() {
            plan.cell_names.insert(
                rcgjs_object::jir::repr::CellId(index as u32),
                format!("c{index}"),
            );
        }
        let body = body_of(BodyKind::Function, func.sig.clone());
        let js = lower(interns, name, &["keep"], &body, func, &plan, straight(func));
        self.add(name, &js, op, width);
        js
    }

    fn run(mut self) {
        self.source.push_str(
            "for (const [name, f, a, b, wrapped, overflowed] of cases) {\n  const [r, o] = f(a, b);\n  \
             assert.ok(r === wrapped && o === overflowed, `${name}(${a}, ${b}) = [${r}, ${o}], \
             want [${wrapped}, ${overflowed}]`);\n}\nconsole.log(cases.length);\n",
        );
        // The program is too long for an argument; it goes in on stdin.
        let output = test_node::run(test_node::Node::Stdin(&self.source))
            .expect("Node is required by the compiler semantic tests");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

const OVERFLOW_OPS: [(BinaryOp, &str); 3] = [
    (BinaryOp::AddWithOverflow, "add"),
    (BinaryOp::SubWithOverflow, "sub"),
    (BinaryOp::MulWithOverflow, "mul"),
];

#[test]
fn an_escaping_overflow_pair_is_an_array_without_a_closure() {
    let interns = world();
    let func = escaping_overflow(BinaryOp::AddWithOverflow, U32, OVERFLOW);
    let plan = plan_of(&func, &[(0, "a"), (1, "b")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(a, b) {\n  let $t0, $t1;\n  return [$t1 = ($t0 = a + b) >>> 0, $t1 !== $t0];\n}\n"
    );
}

#[test]
fn an_escaping_32_bit_product_wraps_through_imul() {
    let interns = world();
    let func = escaping_overflow(BinaryOp::MulWithOverflow, U32, OVERFLOW);
    let plan = plan_of(&func, &[(0, "a"), (1, "b")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(a, b) {\n  let $t0;\n  return [$t0 = Math.imul(a, b) >>> 0, $t0 !== a * b];\n}\n"
    );
}

#[test]
fn a_projected_overflow_pair_is_its_exact_result() {
    let (interns, widths) = overflow_world();
    let func = projected_overflow(BinaryOp::AddWithOverflow, I32, widths[4].pair);
    let plan = plan_of(&func, &[(0, "a"), (1, "b"), (2, "p"), (3, "r"), (4, "o")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(a, b) {\n  let p, r, o;\n  p = a + b;\n  r = p | 0;\n  o = (p | 0) !== p;\n  return [r, o];\n}\n"
    );
}

#[test]
fn an_inline_projected_overflow_pair_binds_one_temporary() {
    let (mut interns, widths) = overflow_world();
    let func = one_half_overflow(BinaryOp::SubWithOverflow, widths[5], 1);
    let plan = plan_of(&func, &[(0, "a"), (1, "b")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        "function f(a, b) {\n  let a$lo, a$hi, b$lo, b$hi, $t0, $t1, $t2;\n  a$lo = Number(BigInt.asIntN(32, a)) | 0;\n  a$hi = Number(BigInt.asIntN(32, a >> 32n)) | 0;\n  b$lo = Number(BigInt.asIntN(32, b)) | 0;\n  b$hi = Number(BigInt.asIntN(32, b >> 32n)) | 0;\n  $t0 = a$lo - b$lo | 0;\n  $t1 = a$lo >>> 0 < b$lo >>> 0 ? 1 : 0;\n  $t2 = a$hi - b$hi - $t1 | 0;\n  return ((a$hi ^ b$hi) & (a$hi ^ $t2)) < 0;\n}\n"
    );
    // 128-bit integers remain BigInt-based and preserve the single-temporary v1 pattern.
    const I128: TypeId = TypeId(63);
    const I128_PAIR: TypeId = TypeId(64);
    interns.intern_type(int(I128, true, 128));
    let mut pair128 = descriptor(I128_PAIR, TypeKind::Tuple(vec![I128, BOOL]), 32);
    pair128.js.is_indirect = true;
    pair128.js.needs_clone = true;
    pair128.js.fields = vec![
        field(FieldKey::Index(0), I128),
        field(FieldKey::Index(1), BOOL),
    ];
    interns.intern_type(pair128);
    let func128 = one_half_overflow(
        BinaryOp::SubWithOverflow,
        Width {
            element: I128,
            pair: I128_PAIR,
            nest: I128_PAIR,
            reference: I128_PAIR,
            raw: I128_PAIR,
            signed: true,
            bits: 128,
        },
        1,
    );
    let plan128 = plan_of(&func128, &[(0, "a"), (1, "b")]);
    let body128 = body_of(BodyKind::Function, func128.sig.clone());
    let js128 = lower(
        &interns,
        "f128",
        &[],
        &body128,
        &func128,
        &plan128,
        straight(&func128),
    );
    assert_eq!(
        js128,
        "function f128(a, b) {\n  let $t0;\n  return BigInt.asIntN(128, $t0 = a - b) !== $t0;\n}\n"
    );
}

#[test]
fn a_32_bit_product_pair_is_built_when_both_halves_are_read() {
    let interns = world();
    let func = projected_overflow(BinaryOp::MulWithOverflow, U32, OVERFLOW);
    let plan = plan_of(&func, &[(0, "a"), (1, "b"), (2, "p"), (3, "r"), (4, "o")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert!(
        js.contains("p = [$t0 = Math.imul(a, b) >>> 0, $t0 !== a * b];")
            && js.contains("r = p[0];")
            && js.contains("o = p[1];"),
        "{js}"
    );
}

/// `a.op(b).<field>` alone, returned as a `bool` (`.1`) or `T` (`.0`).
fn one_half_overflow(op: BinaryOp, width: Width, half: u32) -> Func {
    let ty = width.element;
    let result = if half == 0 { ty } else { BOOL };
    let mut b = FuncBuilder::new(item(1), signature(vec![ty, ty], result));
    let x = b.param(ty, NO_ORIGIN);
    let y = b.param(ty, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let p = b.op(
        e,
        OpKind::Binary { op, l: x, r: y, ty },
        width.pair,
        NO_ORIGIN,
    );
    let read = b.op(
        e,
        OpKind::Field {
            base: p,
            variant: None,
            field: half,
        },
        result,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(read)));
    b.finish()
}

#[test]
fn a_32_bit_product_read_through_one_half_is_that_half_alone() {
    let (interns, widths) = overflow_world();
    let spell = |half, named: &[(u32, &str)]| {
        let func = one_half_overflow(BinaryOp::MulWithOverflow, widths[4], half);
        let plan = plan_of(&func, named);
        let body = body_of(BodyKind::Function, func.sig.clone());
        lower(&interns, "f", &[], &body, &func, &plan, straight(&func))
    };
    assert_eq!(
        spell(0, &[(0, "a"), (1, "b")]),
        "function f(a, b) {\n  return Math.imul(a, b) | 0;\n}\n"
    );
    assert_eq!(
        spell(1, &[(0, "a"), (1, "b")]),
        "function f(a, b) {\n  return (Math.imul(a, b) | 0) !== a * b;\n}\n"
    );
    assert_eq!(
        spell(1, &[(0, "a"), (1, "b"), (2, "p")]),
        "function f(a, b) {\n  let p;\n  p = (Math.imul(a, b) | 0) !== a * b;\n  return p;\n}\n"
    );
}

/// `let p = a.op(b); let q = p (Move or Copy); (q.0, p.1)`: a pair renamed or copied and then
/// only projected. Values: `a b p o q r out`.
fn aliased_overflow(op: BinaryOp, width: Width, copy: bool) -> Func {
    let Width {
        element: ty, pair, ..
    } = width;
    let mut b = FuncBuilder::new(item(1), signature(vec![ty, ty], pair));
    let x = b.param(ty, NO_ORIGIN);
    let y = b.param(ty, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let p = b.op(e, OpKind::Binary { op, l: x, r: y, ty }, pair, NO_ORIGIN);
    let projection = |b: &mut FuncBuilder, base, field, ty| {
        b.op(
            e,
            OpKind::Field {
                base,
                variant: None,
                field,
            },
            ty,
            NO_ORIGIN,
        )
    };
    let overflowed = projection(&mut b, p, 1, BOOL);
    let alias = if copy {
        OpKind::Copy(p)
    } else {
        OpKind::Move(p)
    };
    let q = b.op(e, alias, pair, NO_ORIGIN);
    let wrapped = projection(&mut b, q, 0, ty);
    let out = b.op(
        e,
        OpKind::Aggregate {
            ty: pair,
            variant: 0,
            active_field: None,
            fields: vec![wrapped, overflowed],
        },
        pair,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(out)));
    b.finish()
}

#[test]
fn a_moved_or_copied_projected_pair_is_still_its_exact_result() {
    let (interns, widths) = overflow_world();
    for copy in [false, true] {
        let func = aliased_overflow(BinaryOp::AddWithOverflow, widths[4], copy);
        let plan = plan_of(
            &func,
            &[(0, "a"), (1, "b"), (2, "p"), (3, "o"), (4, "q"), (5, "r")],
        );
        let body = body_of(BodyKind::Function, func.sig.clone());
        let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
        assert_eq!(
            js,
            "function f(a, b) {\n  let p, o, q, r;\n  p = a + b;\n  o = (p | 0) !== p;\n  q = p;\n  \
             r = q | 0;\n  return [r, o];\n}\n",
            "copy: {copy}"
        );
    }
}

/// Every carried and built pair shape of every overflow operator at every width, under Node at
/// the boundary values against Rust's own `overflowing_*`: projected (both halves, carried as the
/// exact result or, for a 32 bit product, built), one half only, moved or copied, and escaping
/// whole.
#[test]
fn overflow_pairs_agree_with_rust_at_every_width() {
    let (interns, widths) = overflow_world();
    let mut harness = OverflowHarness::new();
    let projected: &[(u32, &str)] = &[(0, "a"), (1, "b"), (2, "p"), (3, "r"), (4, "o")];
    let aliased: &[(u32, &str)] = &[(0, "a"), (1, "b"), (2, "p"), (3, "o"), (4, "q"), (5, "r")];
    for width in widths {
        for (op, spelled) in OVERFLOW_OPS {
            let label = format!("{spelled}_{}", width.label());
            for (shape, func, names) in [
                (
                    "projected",
                    projected_overflow(op, width.element, width.pair),
                    projected,
                ),
                (
                    "escaping",
                    escaping_overflow(op, width.element, width.pair),
                    projected,
                ),
                ("moved", aliased_overflow(op, width, false), aliased),
                ("copied", aliased_overflow(op, width, true), aliased),
            ] {
                harness.lower(
                    &interns,
                    &format!("{label}_{shape}"),
                    &func,
                    names,
                    op,
                    width,
                );
            }
            // One half alone, wrapped back into a pair beside the half the reference computes.
            for half in [0, 1] {
                let func = one_half_overflow(op, width, half);
                let name = format!("{label}_half{half}");
                let plan = plan_of(&func, &[(0, "a"), (1, "b")]);
                let body = body_of(BodyKind::Function, func.sig.clone());
                let js = lower(&interns, &name, &[], &body, &func, &plan, straight(&func));
                let reference = format!("{label}_projected");
                let wrapper = if half == 0 {
                    format!("function {name}$w(a, b) {{ return [{name}(a, b), {reference}(a, b)[1]]; }}\n")
                } else {
                    format!("function {name}$w(a, b) {{ return [{reference}(a, b)[0], {name}(a, b)]; }}\n")
                };
                harness.add(&format!("{name}$w"), &format!("{js}{wrapper}"), op, width);
            }
        }
    }
    harness.run();
}

/// A chain of aliases whose blocks are not in dominance order: the block that comes last in
/// control flow is stored first and holds `r = move q; s = copy r` and both projections, while
/// the block stored after it holds the overflow op and `q = move p`. One pass over the blocks sees
/// `r` and `s` before `q` joins the family, so the family must be found by a fixpoint; a carrier
/// committed for only part of the chain would be projected as if it were a pair.
#[test]
fn an_alias_chain_is_one_family_whatever_the_block_order() {
    let (interns, widths) = overflow_world();
    let mut harness = OverflowHarness::new();
    for width in widths {
        for (op, spelled) in OVERFLOW_OPS {
            let Width {
                element: ty, pair, ..
            } = width;
            let mut b = FuncBuilder::new(item(1), signature(vec![ty, ty], pair));
            let x = b.param(ty, NO_ORIGIN);
            let y = b.param(ty, NO_ORIGIN);
            let e = FuncBuilder::ENTRY;
            let last = b.block();
            let middle = b.block();
            b.terminate(e, RTerm::Goto(middle, vec![]));
            let p = b.op(
                middle,
                OpKind::Binary { op, l: x, r: y, ty },
                pair,
                NO_ORIGIN,
            );
            let q = b.op(middle, OpKind::Move(p), pair, NO_ORIGIN);
            b.terminate(middle, RTerm::Goto(last, vec![]));
            let r = b.op(last, OpKind::Move(q), pair, NO_ORIGIN);
            let s = b.op(last, OpKind::Copy(r), pair, NO_ORIGIN);
            let wrapped = b.op(
                last,
                OpKind::Field {
                    base: s,
                    variant: None,
                    field: 0,
                },
                ty,
                NO_ORIGIN,
            );
            let overflowed = b.op(
                last,
                OpKind::Field {
                    base: r,
                    variant: None,
                    field: 1,
                },
                BOOL,
                NO_ORIGIN,
            );
            let out = b.op(
                last,
                OpKind::Aggregate {
                    ty: pair,
                    variant: 0,
                    active_field: None,
                    fields: vec![wrapped, overflowed],
                },
                pair,
                NO_ORIGIN,
            );
            b.terminate(last, RTerm::Return(Some(out)));
            let func = b.finish();
            let plan = plan_of(
                &func,
                &[
                    (0, "a"),
                    (1, "b"),
                    (2, "p"),
                    (3, "q"),
                    (4, "r"),
                    (5, "s"),
                    (6, "w"),
                    (7, "o"),
                ],
            );
            let region = Region::Seq(vec![
                Region::Block(payload(&func, 0, None)),
                Region::Block(payload(&func, middle.0, None)),
                Region::Block(payload(&func, last.0, None)),
                Region::Return,
            ]);
            let name = format!("{spelled}_{}_out_of_order", width.label());
            let body = body_of(BodyKind::Function, func.sig.clone());
            let js = lower(&interns, &name, &[], &body, &func, &plan, region);
            harness.add(&name, &js, op, width);
        }
    }
    harness.run();
}

/// How a pair escapes in [`escaping_use`]: each is a use `carried_overflow_pairs` must refuse.
#[derive(Clone, Copy, Debug)]
enum Escape {
    /// `q = keep(p)`, a call argument.
    Call,
    /// `w = (p,)` and `q = w.0`, an aggregate field.
    Aggregate,
    /// `c = p; q = c`, a store to a cell.
    CellStore,
    /// `w = (seed,); w.0 = p; q = w.0`, a field store.
    SetField,
    /// `c = seed; *&mut c = p; q = c`, a store through a pointer.
    StoreThrough,
    /// `q = *&p`, a reference view.
    Borrow,
    /// `q = *(&raw const p)`, a raw view.
    Raw,
    /// `q = copy p; keep(q)`, a copy whose own use escapes.
    CopyThenCall,
}

impl Escape {
    const ALL: [Escape; 8] = [
        Escape::Call,
        Escape::Aggregate,
        Escape::CellStore,
        Escape::SetField,
        Escape::StoreThrough,
        Escape::Borrow,
        Escape::Raw,
        Escape::CopyThenCall,
    ];
}

/// `let p = a.op(b); let o = p.1; let q = <p through escape>; (q.0, o)`, with the plan's names.
/// The projection `o` makes `p` a pair that would be carried if the escape were not seen.
fn escaping_use(
    op: BinaryOp,
    width: Width,
    escape: Escape,
    keep: StrId,
) -> (Func, Vec<(u32, String)>) {
    let Width {
        element: ty,
        pair,
        nest,
        reference,
        raw,
        ..
    } = width;
    let mut b = FuncBuilder::new(item(1), signature(vec![ty, ty], pair));
    let x = b.param(ty, NO_ORIGIN);
    let y = b.param(ty, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let p = b.op(e, OpKind::Binary { op, l: x, r: y, ty }, pair, NO_ORIGIN);
    let field = |b: &mut FuncBuilder, base, field, ty| {
        b.op(
            e,
            OpKind::Field {
                base,
                variant: None,
                field,
            },
            ty,
            NO_ORIGIN,
        )
    };
    let call = |b: &mut FuncBuilder, arg| {
        b.op(
            e,
            OpKind::Call {
                callee: Callee::Direct {
                    item: item(9),
                    link_name: keep,
                },
                function: None,
                args: vec![arg],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            pair,
            NO_ORIGIN,
        )
    };
    let overflowed = field(&mut b, p, 1, BOOL);
    // A pair value distinct from `p`, for the stores to overwrite.
    let seed = |b: &mut FuncBuilder| {
        b.op(
            e,
            OpKind::Aggregate {
                ty: pair,
                variant: 0,
                active_field: None,
                fields: vec![x, overflowed],
            },
            pair,
            NO_ORIGIN,
        )
    };
    let nested = |b: &mut FuncBuilder, inner| {
        b.op(
            e,
            OpKind::Aggregate {
                ty: nest,
                variant: 0,
                active_field: None,
                fields: vec![inner],
            },
            nest,
            NO_ORIGIN,
        )
    };
    let q = match escape {
        Escape::Call => call(&mut b, p),
        Escape::Aggregate => {
            let w = nested(&mut b, p);
            field(&mut b, w, 0, pair)
        }
        Escape::CellStore => {
            let cell = b.cell(pair, false, NO_ORIGIN);
            b.effect(e, OpKind::CellAlloc(cell), NO_ORIGIN);
            b.effect(e, OpKind::CellStore { cell, v: p }, NO_ORIGIN);
            b.op(e, OpKind::CellLoad(cell), pair, NO_ORIGIN)
        }
        Escape::SetField => {
            let start = seed(&mut b);
            let w = nested(&mut b, start);
            b.effect(
                e,
                OpKind::SetField {
                    base: w,
                    variant: None,
                    field: 0,
                    v: p,
                },
                NO_ORIGIN,
            );
            field(&mut b, w, 0, pair)
        }
        Escape::StoreThrough => {
            let start = seed(&mut b);
            let cell = b.cell(pair, true, NO_ORIGIN);
            b.effect(e, OpKind::CellAlloc(cell), NO_ORIGIN);
            b.effect(e, OpKind::CellStore { cell, v: start }, NO_ORIGIN);
            let slot = b.op(
                e,
                OpKind::BorrowView {
                    root: Root::Cell(cell),
                    path: vec![],
                    mutable: true,
                },
                reference,
                NO_ORIGIN,
            );
            b.effect(e, OpKind::StoreThrough { ptr: slot, v: p }, NO_ORIGIN);
            b.op(e, OpKind::CellLoad(cell), pair, NO_ORIGIN)
        }
        Escape::Borrow | Escape::Raw => {
            let (kind, ty) = match escape {
                Escape::Borrow => (
                    OpKind::BorrowView {
                        root: Root::Value(p),
                        path: vec![],
                        mutable: false,
                    },
                    reference,
                ),
                _ => (
                    OpKind::RawView {
                        root: Root::Value(p),
                        path: vec![],
                        mutable: false,
                    },
                    raw,
                ),
            };
            let view = b.op(e, kind, ty, NO_ORIGIN);
            b.op(e, OpKind::Deref(view), pair, NO_ORIGIN)
        }
        Escape::CopyThenCall => {
            let copy = b.op(e, OpKind::Copy(p), pair, NO_ORIGIN);
            call(&mut b, copy)
        }
    };
    let wrapped = field(&mut b, q, 0, ty);
    let out = b.op(
        e,
        OpKind::Aggregate {
            ty: pair,
            variant: 0,
            active_field: None,
            fields: vec![wrapped, overflowed],
        },
        pair,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(out)));
    let func = b.finish();
    // Every value but a view is named, so a value used twice is never spelled twice.
    let special = [(p, "p"), (overflowed, "o"), (q, "q"), (wrapped, "r")];
    let names = func.blocks[0]
        .ops
        .iter()
        .filter(|op| !matches!(op.kind, OpKind::BorrowView { .. } | OpKind::RawView { .. }))
        .filter_map(|op| op.result)
        .map(|v| {
            let name = special
                .iter()
                .find(|(value, _)| *value == v)
                .map_or_else(|| format!("v{}", v.0), |(_, name)| (*name).to_owned());
            (v.0, name)
        })
        .chain([(x.0, "a".to_owned()), (y.0, "b".to_owned())])
        .collect();
    (func, names)
}

/// A pair that escapes through a call argument, an aggregate field, a store (to a cell, a field,
/// through a pointer), a view (reference or raw) or a copy that escapes in turn is built as a
/// real array — even though it is also projected — and still agrees with Rust under Node.
#[test]
fn an_overflow_pair_that_escapes_through_any_use_is_built() {
    let (mut interns, widths) = overflow_world();
    let keep = interns.intern_str("keep");
    let mut harness = OverflowHarness::new();
    for width in widths {
        for (op, spelled) in OVERFLOW_OPS {
            for escape in Escape::ALL {
                let (func, names) = escaping_use(op, width, escape, keep);
                let names: Vec<(u32, &str)> =
                    names.iter().map(|(v, name)| (*v, name.as_str())).collect();
                let name = format!("{spelled}_{}_{escape:?}", width.label());
                let js = harness.lower(&interns, &name, &func, &names, op, width);
                let built = if width.bits == 64 {
                    js.contains("p = [BigInt(")
                } else {
                    js.contains("p = [$t")
                };
                assert!(built, "{name}: the escaping pair must be built\n{js}");
            }
        }
    }
    harness.run();
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
        "function g(a, n, f) {\n  let a$lo, a$hi, $t0, $t1, $t2, $t3, $t4, $t5, $t6;\n  a$lo = Number(BigInt.asIntN(32, a)) | 0;\n  a$hi = Number(BigInt.asIntN(32, a >> 32n)) | 0;\n  $t0 = n & 63;\n  $t1 = n | 0;\n  $t2 = ($t0 & 32) !== 0 ? 0 : a$lo << $t0;\n  $t3 = $t2 + $t1 | 0;\n  $t4 = $__mires_i64_f2i(Math.fround(f * f), true);\n  $t5 = $__mires_i64_hi;\n  $t6 = -($t3 - $t4 | 0) | 0;\n  return BigInt(~(((($t0 & 32) !== 0 ? a$lo << $t0 : a$hi << $t0 | a$lo >>> 1 >>> 31 - $t0) + ($t1 >> 31) + ($t3 >>> 0 < $t2 >>> 0 ? 1 : 0) | 0) - $t5 - ($t3 >>> 0 < $t4 >>> 0 ? 1 : 0) | 0) + ($t6 === 0 ? 1 : 0) | 0) << 32n | BigInt($t6 >>> 0);\n}\n"
    );
    // 128-bit integers continue to lower to BigInt arithmetic like v1.
    let mut interns128 = world();
    const I128: TypeId = TypeId(63);
    interns128.intern_type(int(I128, true, 128));
    let mut b128 = FuncBuilder::new(item(1), signature(vec![I128, I32, F32], I128));
    let a128 = b128.param(I128, NO_ORIGIN);
    let n128 = b128.param(I32, NO_ORIGIN);
    let f128 = b128.param(F32, NO_ORIGIN);
    let e128 = FuncBuilder::ENTRY;
    let shl128 = b128.op(
        e128,
        OpKind::Binary {
            op: BinaryOp::Shl,
            l: a128,
            r: n128,
            ty: I128,
        },
        I128,
        NO_ORIGIN,
    );
    let widened128 = b128.op(
        e128,
        OpKind::Cast {
            kind: CastKind::IntToInt,
            v: n128,
            from: I32,
            to: I128,
        },
        I128,
        NO_ORIGIN,
    );
    let sum128 = b128.op(
        e128,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: shl128,
            r: widened128,
            ty: I128,
        },
        I128,
        NO_ORIGIN,
    );
    let sq128 = b128.op(
        e128,
        OpKind::Binary {
            op: BinaryOp::Mul,
            l: f128,
            r: f128,
            ty: F32,
        },
        F32,
        NO_ORIGIN,
    );
    let fi128 = b128.op(
        e128,
        OpKind::Cast {
            kind: CastKind::FloatToInt,
            v: sq128,
            from: F32,
            to: I128,
        },
        I128,
        NO_ORIGIN,
    );
    let total128 = b128.op(
        e128,
        OpKind::Binary {
            op: BinaryOp::Sub,
            l: sum128,
            r: fi128,
            ty: I128,
        },
        I128,
        NO_ORIGIN,
    );
    let neg128 = b128.op(
        e128,
        OpKind::Unary {
            op: UnaryOp::Neg,
            v: total128,
            ty: I128,
        },
        I128,
        NO_ORIGIN,
    );
    b128.terminate(e128, RTerm::Return(Some(neg128)));
    let func128 = b128.finish();
    let plan128 = plan_of(&func128, &[(0, "a"), (1, "n"), (2, "f")]);
    let body128 = body_of(BodyKind::Function, func128.sig.clone());
    let js128 = lower(
        &interns128,
        "g128",
        &[],
        &body128,
        &func128,
        &plan128,
        straight(&func128),
    );
    assert_eq!(
        js128,
        "function g128(a, n, f) {\n  return BigInt.asIntN(128, -BigInt.asIntN(128, BigInt.asIntN(128, BigInt.asIntN(128, a << (BigInt(n) & 127n)) + BigInt.asIntN(128, BigInt(n))) - __rt.f2i_big(Math.fround(f * f), 128, true)));\n}\n"
    );
}

/// `a >> 33u32` at i128 (`IntRepr::BigInt`), lowered as phase C lowers it with BigInt constants
/// hoisted, and as every other lowering (the cost trials, `js-ecma-target=latest`) lowers it.
#[test]
fn a_hoisting_lowering_names_its_bigint_constants_and_widens_constant_shift_amounts() {
    let mut interns = world();
    const I128: TypeId = TypeId(63);
    interns.intern_type(int(I128, true, 128));
    let mut b = FuncBuilder::new(item(1), signature(vec![I128], I128));
    let a = b.param(I128, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let amount = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 33, bytes: 4 }),
        U32,
        NO_ORIGIN,
    );
    let shifted = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Shr,
            l: a,
            r: amount,
            ty: I128,
        },
        I128,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(shifted)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "a")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let names =
        crate::lower::bigint::BigIntNames::allocate(&std::collections::BTreeMap::new(), &[])
            .expect("an empty program spells no prefix");
    let opts = Opts::default();
    let name = JsName::new("g".to_owned());
    let summary = summary(&[]);
    let lowered = |bigints| {
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
            bigints,
        };
        match function::lower(&input, &func, &plan, straight(&func)) {
            Ok(item) => item,
            Err(decline) => panic!("declined: {decline}"),
        }
    };

    let inline = lowered(None);
    assert_eq!(
        jsast::stmt_to_string(&inline.decl),
        "function g(a) {\n  return BigInt.asIntN(128, a >> (BigInt(33) & 127n));\n}\n"
    );
    assert!(inline.bigints.is_empty());

    let hoisted = lowered(Some(&names));
    assert_eq!(
        jsast::stmt_to_string(&hoisted.decl),
        "function g(a) {\n  return BigInt.asIntN(128, a >> (($BI_ok ? $BI_33 : $BI_no()) & $BI_127));\n}\n"
    );
    assert_eq!(
        hoisted.bigints,
        std::collections::BTreeSet::from(["127".to_owned(), "33".to_owned()])
    );
    // The bindings, the `BigInt` test and the throwing function are references like any other,
    // which is what the lane driver prepares.
    for name in ["$BI_33", "$BI_127", "$BI_ok", "$BI_no"] {
        assert!(
            hoisted.refs.contains(&JsName::new(name.to_owned())),
            "{name}"
        );
    }
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
        bigints: None,
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
        bigints: None,
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

/// Lowers `func` with the real fusion plan, returning the plan and the printed function. `deps`
/// are the callees it may name.
fn planned(interns: &Interns, func: &Func, deps: &[&str]) -> (Plan, String) {
    let plan = super::super::fuse::plan(func, None, &interns.types, interns, &Opts::default());
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(interns, "f", deps, &body, func, &plan, straight(func));
    (plan, js)
}

#[test]
fn a_copy_written_and_never_read_again_keeps_the_write() {
    let interns = world();
    // `fn f(p: &Point) -> i32 { let mut q = *p; q.x = 5; p.x }`: the copy's one use is the write.
    // Spelled inline, its clone `{ x: $v1.x, y: $v1.y }` folded under the field write to
    // `$v1.x = 5`, which is the caller's point.
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_POINT], I32));
    let p = b.param(REF_POINT, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let deref = b.op(e, OpKind::Deref(p), POINT, NO_ORIGIN);
    let q = b.op(e, OpKind::Copy(deref), POINT, NO_ORIGIN);
    let five = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::SetField {
            base: q,
            variant: None,
            field: 0,
            v: five,
        },
        NO_ORIGIN,
    );
    let again = b.op(e, OpKind::Deref(p), POINT, NO_ORIGIN);
    let x = b.op(
        e,
        OpKind::Field {
            base: again,
            variant: None,
            field: 0,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(x)));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &[]);
    assert!(!plan.inline.contains(&q));
    assert_eq!(
        js,
        "function f($v0) {\n  let $v1, $v2;\n  $v1 = $v0;\n  $v2 = { x: $v1.x, y: $v1.y };\n  $v2.x = 5;\n  return $v0.x;\n}\n"
    );
}

#[test]
fn an_aggregate_written_and_never_read_again_keeps_its_operands() {
    let interns = world();
    // `fn f(a: i32, c: i32) -> i32 { let mut q = Point { x: a, y: c }; q.x = 5; a }`: inline, the
    // literal's field folded under the write to its operand, and the write became `a = 5`.
    let mut b = FuncBuilder::new(item(1), signature(vec![I32, I32], I32));
    let a = b.param(I32, NO_ORIGIN);
    let c = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let q = b.op(
        e,
        OpKind::Aggregate {
            ty: POINT,
            variant: 0,
            active_field: None,
            fields: vec![a, c],
        },
        POINT,
        NO_ORIGIN,
    );
    let five = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::SetField {
            base: q,
            variant: None,
            field: 0,
            v: five,
        },
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(a)));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &[]);
    assert!(!plan.inline.contains(&q));
    assert_eq!(
        js,
        "function f($v0, $v1) {\n  let $v2;\n  $v2 = { x: $v0, y: $v1 };\n  $v2.x = 5;\n  return $v0;\n}\n"
    );
}

#[test]
fn a_view_into_a_copy_is_rooted_at_the_copy() {
    let interns = world();
    // `fn f(p: &Point) -> i32 { let mut q = *p; *(&mut q.x) = 5; p.x }`: the view designates the
    // copy's storage, so the copy is declared and the store lands in it.
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_POINT], I32));
    let p = b.param(REF_POINT, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let deref = b.op(e, OpKind::Deref(p), POINT, NO_ORIGIN);
    let q = b.op(e, OpKind::Copy(deref), POINT, NO_ORIGIN);
    let view = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(q),
            path: vec![Proj::Field { field: 0, ty: I32 }],
            mutable: true,
        },
        REF_I32,
        NO_ORIGIN,
    );
    let five = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::StoreThrough { ptr: view, v: five }, NO_ORIGIN);
    let again = b.op(e, OpKind::Deref(p), POINT, NO_ORIGIN);
    let x = b.op(
        e,
        OpKind::Field {
            base: again,
            variant: None,
            field: 0,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(x)));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &[]);
    assert!(!plan.inline.contains(&q));
    assert_eq!(
        js,
        "function f($v0) {\n  let $v1, $v2;\n  $v1 = $v0;\n  $v2 = { x: $v1.x, y: $v1.y };\n  $v2.x = 5;\n  return $v0.x;\n}\n"
    );
}

/// `f(a, c)`'s literal `Point { x: a, y: c }` and the constant `5`, the value and the store the
/// tests below write into it.
fn point_and_five(b: &mut FuncBuilder, a: ValueId, c: ValueId) -> (ValueId, ValueId) {
    let e = FuncBuilder::ENTRY;
    let q = b.op(
        e,
        OpKind::Aggregate {
            ty: POINT,
            variant: 0,
            active_field: None,
            fields: vec![a, c],
        },
        POINT,
        NO_ORIGIN,
    );
    let five = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    (q, five)
}

#[test]
fn a_shared_view_into_an_aggregate_is_rooted_at_the_aggregate() {
    let mut interns = world();
    let put = interns.intern_str("put");
    // `fn f(a: i32, c: i32) -> i32 { let q = C { x: Cell::new(a), y: c }; q.x.set(5); a }` with
    // the `Cell` peeled to its `i32`: `put` (`Cell::set`) writes through the SHARED reference it
    // is handed, so the reference designates storage like a mutable one. Spelled inline, it was a
    // slot into the literal with no container of its own, and the lowering declined the item.
    let mut b = FuncBuilder::new(item(1), signature(vec![I32, I32], I32));
    let a = b.param(I32, NO_ORIGIN);
    let c = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let (q, five) = point_and_five(&mut b, a, c);
    let view = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(q),
            path: vec![Proj::Field { field: 0, ty: I32 }],
            mutable: false,
        },
        REF_I32,
        NO_ORIGIN,
    );
    b.effect(e, call_op(put, vec![view, five]), NO_ORIGIN);
    b.terminate(e, RTerm::Return(Some(a)));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &["put"]);
    assert!(!plan.inline.contains(&q));
    assert_eq!(
        js,
        "function f($v0, $v1) {\n  let $v2;\n  $v2 = { x: $v0, y: $v1 };\n  put({ buf: $v2, off: \"x\" }, 5);\n  return $v0;\n}\n"
    );
}

#[test]
fn a_raw_view_into_an_aggregate_is_rooted_at_the_aggregate() {
    let interns = world();
    // `fn f(a: i32, c: i32) -> i32 { let q = U { x: UnsafeCell::new(a), y: c };
    // *UnsafeCell::raw_get(&raw const q.x) = 5; a }` once `raw_get` is inlined: a `*const` view
    // of the field, stored through. Spelled inline, the store's place folded through the literal
    // to the literal's operand and wrote the parameter, `$v0 = 5`.
    let mut b = FuncBuilder::new(item(1), signature(vec![I32, I32], I32));
    let a = b.param(I32, NO_ORIGIN);
    let c = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let (q, five) = point_and_five(&mut b, a, c);
    let raw = b.op(
        e,
        OpKind::RawView {
            root: Root::Value(q),
            path: vec![Proj::Field { field: 0, ty: I32 }],
            mutable: false,
        },
        RAW_I32,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::StoreThrough { ptr: raw, v: five }, NO_ORIGIN);
    b.terminate(e, RTerm::Return(Some(a)));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &[]);
    assert!(!plan.inline.contains(&q));
    assert_eq!(
        js,
        "function f($v0, $v1) {\n  let $v2;\n  $v2 = { x: $v0, y: $v1 };\n  $v2.x = 5;\n  return $v0;\n}\n"
    );
}

#[test]
fn a_variant_set_on_an_aggregate_is_set_on_the_aggregate() {
    let interns = world();
    // `fn f(a: i32, c: i32) -> i32 { let mut q = Shape::Pair(a, c); <q's variant := Num>; a }`:
    // the `SetDiscriminant` is the literal's one use. Spelled inline, the tag write folded
    // through the literal to its `TAG` entry and assigned to a string.
    let mut b = FuncBuilder::new(item(1), signature(vec![I32, I32], I32));
    let a = b.param(I32, NO_ORIGIN);
    let c = b.param(I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let q = b.op(
        e,
        OpKind::Aggregate {
            ty: SHAPE,
            variant: 2,
            active_field: None,
            fields: vec![a, c],
        },
        SHAPE,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::SetDiscriminant {
            base: q,
            variant: 1,
        },
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(a)));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &[]);
    assert!(!plan.inline.contains(&q));
    assert_eq!(
        js,
        "function f($v0, $v1) {\n  let $v2;\n  $v2 = { TAG: \"Pair\", _0: $v0, _1: $v1 };\n  $v2.TAG = \"Num\";\n  return $v0;\n}\n"
    );
}

/// The positive controls: what is only read, or only reached through a pointer it holds, is still
/// spelled inline.
#[test]
fn a_copy_read_or_dereferenced_is_still_spelled_inline() {
    let interns = world();
    // `fn f(p: &Point) -> i32 { let q = *p; q.x }`: the copy is read, so its clone is spelled at
    // the read and the field read folds through it.
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_POINT], I32));
    let p = b.param(REF_POINT, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let deref = b.op(e, OpKind::Deref(p), POINT, NO_ORIGIN);
    let q = b.op(e, OpKind::Copy(deref), POINT, NO_ORIGIN);
    let x = b.op(
        e,
        OpKind::Field {
            base: q,
            variant: None,
            field: 0,
        },
        I32,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(x)));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &[]);
    assert!(plan.inline.contains(&q));
    assert_eq!(
        js,
        "function f($v0) {\n  let $v1;\n  $v1 = $v0;\n  return $v1.x;\n}\n"
    );

    // `fn f(p: &mut Point) { let r = p; (*r).x = 5 }`: the view dereferences the copied
    // reference before it reaches storage, so it designates the pointee, not the copy.
    let mut b = FuncBuilder::new(item(1), signature(vec![REF_POINT], UNIT));
    let p = b.param(REF_POINT, NO_ORIGIN);
    let r = b.op(e, OpKind::Copy(p), REF_POINT, NO_ORIGIN);
    let view = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Value(r),
            path: vec![Proj::Deref, Proj::Field { field: 0, ty: I32 }],
            mutable: true,
        },
        REF_I32,
        NO_ORIGIN,
    );
    let five = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::StoreThrough { ptr: view, v: five }, NO_ORIGIN);
    b.terminate(e, RTerm::Return(None));
    let func = b.finish();
    let (plan, js) = planned(&interns, &func, &[]);
    assert!(plan.inline.contains(&r));
    assert_eq!(js, "function f($v0) {\n  $v0.x = 5;\n}\n");
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
fn niche_scalar_constants_peel_transparent_and_pattern_fields() {
    // Models `Option<NonZero<u32>>` where `NonZero<u32>` is a transparent wrapper over
    // `NonZeroU32Inner` over `Pattern(u32)`, plus `niche_width` walking `[0, 0, 0]`.
    let mut interns = world();
    let pat_u32 = TypeId(100);
    let nz_inner = TypeId(101);
    let nz_u32 = TypeId(102);
    let opt_nz = TypeId(103);
    let wrap_nz = TypeId(104);
    let tup_nz = TypeId(105);
    let arr_nz = TypeId(106);

    let none = interns.intern_str("None");
    let some = interns.intern_str("Some");
    let f0 = interns.intern_str("_0");
    let inner_name = interns.intern_str("NonZeroU32Inner");
    let nz_name = interns.intern_str("NonZero");
    let wrap_name = interns.intern_str("Wrap");

    let mut pat = descriptor(pat_u32, TypeKind::Pattern(U32), 4);
    pat.js.int = Some(IntFacts {
        signed: false,
        bits: 32,
        repr: IntRepr::Number,
    });
    pat.js.peeled = U32;
    interns.intern_type(pat);

    let mut inner = descriptor(
        nz_inner,
        TypeKind::Adt {
            def: def(101),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                inner_name,
                vec![field(FieldKey::Transparent, pat_u32)],
                0,
                None,
            )],
            niche: None,
        },
        4,
    );
    inner.js.int = Some(IntFacts {
        signed: false,
        bits: 32,
        repr: IntRepr::Number,
    });
    inner.js.transparent = Some(rcgjs_object::jir::types::TransparentField {
        field: 0,
        ty: pat_u32,
    });
    inner.js.peeled = U32;
    interns.intern_type(inner);

    let mut nz = descriptor(
        nz_u32,
        TypeKind::Adt {
            def: def(102),
            args: vec![U32],
            form: NominalForm::Struct,
            variants: vec![variant(
                nz_name,
                vec![field(FieldKey::Transparent, nz_inner)],
                0,
                None,
            )],
            niche: None,
        },
        4,
    );
    nz.js.int = Some(IntFacts {
        signed: false,
        bits: 32,
        repr: IntRepr::Number,
    });
    nz.js.transparent = Some(rcgjs_object::jir::types::TransparentField {
        field: 0,
        ty: nz_inner,
    });
    nz.js.peeled = U32;
    interns.intern_type(nz);

    let opt_niche = rcgjs_object::jir::types::Niche {
        untagged_variant: 1,
        niche_field: vec![0, 0, 0],
        niche_start: 0,
        valid_range: (0, u32::MAX as u128),
    };
    let mut opt = descriptor(
        opt_nz,
        TypeKind::Adt {
            def: def(103),
            args: vec![nz_u32],
            form: NominalForm::Enum,
            variants: vec![
                variant(none, Vec::new(), 0, None),
                variant(some, vec![field(FieldKey::Name(f0), nz_u32)], 1, None),
            ],
            niche: Some(opt_niche.clone()),
        },
        4,
    );
    opt.js.is_indirect = true;
    opt.js.needs_clone = true;
    opt.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(opt);

    let mut wrap = descriptor(
        wrap_nz,
        TypeKind::Adt {
            def: def(104),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                wrap_name,
                vec![field(FieldKey::Name(f0), nz_u32)],
                0,
                None,
            )],
            niche: None,
        },
        4,
    );
    wrap.js.is_indirect = true;
    wrap.js.needs_clone = true;
    interns.intern_type(wrap);

    let mut tup = descriptor(tup_nz, TypeKind::Tuple(vec![nz_u32]), 4);
    tup.js.is_indirect = true;
    tup.js.needs_clone = true;
    interns.intern_type(tup);

    let mut arr = descriptor(
        arr_nz,
        TypeKind::Array {
            element: nz_u32,
            length: 1,
        },
        4,
    );
    arr.js.is_indirect = true;
    arr.js.needs_clone = true;
    interns.intern_type(arr);

    // `niche_width` walks through unpeeled transparent wrappers (`[0, 0, 0]`) to `u32`.
    assert_eq!(
        super::value::niche_width(&interns.types, interns.types.variants(opt_nz), &opt_niche),
        Some(32)
    );

    let check = |ty: TypeId, bits: u128, expected: &str| {
        let mut b = FuncBuilder::new(item(1), signature(vec![], ty));
        let e = FuncBuilder::ENTRY;
        let v = b.op(
            e,
            OpKind::Const(ConstTree::Scalar { bits, bytes: 4 }),
            ty,
            NO_ORIGIN,
        );
        b.terminate(e, RTerm::Return(Some(v)));
        let func = b.finish();
        let plan = plan_of(&func, &[]);
        let body = body_of(BodyKind::Function, func.sig.clone());
        let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
        assert_eq!(js, format!("function f() {{\n  return {expected};\n}}\n"));
    };

    check(opt_nz, 5, "{ TAG: \"Some\", _0: 5 }");
    check(opt_nz, 0, "{ TAG: \"None\" }");
    check(wrap_nz, 7, "{ _0: 7 }");
    check(tup_nz, 9, "[9]");
    check(arr_nz, 11, "[11]");
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
        bigints: None,
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
        bigints: None,
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
        bigints: None,
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
        bigints: None,
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
        bigints: None,
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
        bigints: None,
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
            arguments: Vec::new(),
            caller_location: true,
            caller_location_def: None,
            is_core: false,
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
            arguments: Vec::new(),
            caller_location: false,
            caller_location_def: None,
            is_core: false,
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
            arguments: Vec::new(),
            caller_location: false,
            caller_location_def: None,
            is_core: false,
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
            arguments: Vec::new(),
            caller_location: false,
            caller_location_def: None,
            is_core: false,
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
            bigints: None,
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
fn the_mul_with_overflow_intrinsic_wraps_a_32_bit_product_through_imul() {
    let mut interns = world();
    let name = interns.intern_str("mul_with_overflow");
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(1), signature(vec![U32, U32], OVERFLOW));
    let x = b.param(U32, NO_ORIGIN);
    let y = b.param(U32, NO_ORIGIN);
    let pair = b.op(
        e,
        OpKind::Intrinsic {
            name,
            types: vec![U32],
            fallback: None,
            args: vec![x, y],
        },
        OVERFLOW,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(pair)));
    let func = b.finish();
    let plan = plan_of(&func, &[(0, "x"), (1, "y"), (pair.0, "p")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(&interns, "f", &[], &body, &func, &plan, straight(&func));
    assert_eq!(
        js,
        concat!(
            "function f(x, y) {\n  let p, $t0, $t1;\n  $t0 = x * y;\n",
            "  $t1 = Math.imul(x, y) >>> 0;\n  p = [$t1, $t1 !== $t0];\n  return p;\n}\n"
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

/// `Holder { n: i32, r: &Point }` and `[Point; 2]`, for the tests of a reference read out of a
/// place: the table [`world`] holds, plus a field and an element whose type is or holds a `Point`.
fn reference_holders(interns: &mut Interns) -> (TypeId, TypeId) {
    const HOLDER: TypeId = TypeId(50);
    const POINTS: TypeId = TypeId(51);
    let name = interns.intern_str("Holder");
    let n = interns.intern_str("n");
    let r = interns.intern_str("r");
    let fields = vec![
        field(FieldKey::Name(n), I32),
        field(FieldKey::Name(r), REF_POINT),
    ];
    let mut holder = descriptor(
        HOLDER,
        TypeKind::Adt {
            def: def(40),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(name, fields.clone(), 0, None)],
            niche: None,
        },
        8,
    );
    holder.js.is_indirect = true;
    holder.js.needs_clone = true;
    holder.js.fields = fields;
    interns.intern_type(holder);
    let mut points = descriptor(
        POINTS,
        TypeKind::Array {
            element: POINT,
            length: 2,
        },
        16,
    );
    points.js.is_indirect = true;
    points.js.needs_clone = true;
    points.js.fields = vec![field(FieldKey::Index(0), POINT)];
    interns.intern_type(points);
    (HOLDER, POINTS)
}

/// A direct call of `callee` with `args`, for its effect.
fn call_op(callee: StrId, args: Vec<ValueId>) -> OpKind {
    OpKind::Call {
        callee: Callee::Direct {
            item: item(2),
            link_name: callee,
        },
        function: None,
        args,
        track_caller: None,
        semantic: CallSemantic::Ordinary,
        host: None,
    }
}

/// A reference to an aggregate is the object, and the raw pointer made from it is the object's
/// box, however the reference was read: from a field (`a.r`), as an inline borrow of an element
/// (`xs[1]`), or from a boxed cell (`c[0]`). The slot of that expression would be the address of
/// the storage holding the reference, which `ptr::eq` tells apart from the box of the same object
/// (`examples/known-gaps/201_reference_field_pointers`, `audit_named_reference`). The address of
/// the field itself, and the pointee of a raw pointer, are still the slots they name.
#[test]
fn a_reference_read_out_of_a_place_is_a_raw_pointer_to_its_pointee() {
    let mut interns = world();
    let (holder, points) = reference_holders(&mut interns);
    let same = interns.intern_str("same");
    let ref_to_ref_point = TypeId(52);
    interns.intern_type(pointer(
        ref_to_ref_point,
        REF_POINT,
        PointerForm::Slot,
        true,
    ));

    let mut b = FuncBuilder::new(
        item(1),
        signature(vec![holder, POINT, points, RAW_POINT], UNIT),
    );
    let e = FuncBuilder::ENTRY;
    let a = b.param(holder, NO_ORIGIN);
    let h = b.param(POINT, NO_ORIGIN);
    let xs = b.param(points, NO_ORIGIN);
    let p = b.param(RAW_POINT, NO_ORIGIN);
    let raw_of = |b: &mut FuncBuilder, root: Root| {
        b.op(
            e,
            OpKind::RawView {
                root,
                path: vec![Proj::Deref],
                mutable: false,
            },
            RAW_POINT,
            NO_ORIGIN,
        )
    };
    let borrow = |b: &mut FuncBuilder, root: ValueId, path: Vec<Proj>| {
        b.op(
            e,
            OpKind::BorrowView {
                root: Root::Value(root),
                path,
                mutable: false,
            },
            REF_POINT,
            NO_ORIGIN,
        )
    };

    // `same(a.r, &h)`: the pointer stored in the field, beside a fresh borrow.
    let loaded = b.op(
        e,
        OpKind::Field {
            base: a,
            variant: None,
            field: 1,
        },
        REF_POINT,
        NO_ORIGIN,
    );
    let from_field = raw_of(&mut b, Root::Value(loaded));
    let h_ref = borrow(&mut b, h, Vec::new());
    let from_local = raw_of(&mut b, Root::Value(h_ref));
    b.effect(e, call_op(same, vec![from_field, from_local]), NO_ORIGIN);

    // `same(&xs[1], &a.r as *const &Point)`: an element borrow, and the field's own address.
    let one = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let element = borrow(&mut b, xs, vec![Proj::Index(one)]);
    let from_element = raw_of(&mut b, Root::Value(element));
    let field_address = b.op(
        e,
        OpKind::RawView {
            root: Root::Value(a),
            path: vec![Proj::Field {
                field: 1,
                ty: REF_POINT,
            }],
            mutable: false,
        },
        ref_to_ref_point,
        NO_ORIGIN,
    );
    b.effect(
        e,
        call_op(same, vec![from_element, field_address]),
        NO_ORIGIN,
    );

    // `let c = &h; same(c, p)`, with `c` a boxed cell: a cell read, and a raw pointer's pointee.
    let cell = b.cell(REF_POINT, true, NO_ORIGIN);
    b.effect(e, OpKind::CellAlloc(cell), NO_ORIGIN);
    let h_ref = borrow(&mut b, h, Vec::new());
    b.effect(e, OpKind::CellStore { cell, v: h_ref }, NO_ORIGIN);
    let from_cell = raw_of(&mut b, Root::Cell(cell));
    let through_raw = raw_of(&mut b, Root::Value(p));
    b.effect(e, call_op(same, vec![from_cell, through_raw]), NO_ORIGIN);
    b.terminate(e, RTerm::Return(None));

    let func = b.finish();
    func.verify(&interns.types).unwrap();
    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    // The fix is in the conversion, not a loss of fusion: the loaded reference stays inline.
    assert!(plan.inline.contains(&loaded));
    assert!(plan.inline.contains(&element));
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(
        &interns,
        "f",
        &["same"],
        &body,
        &func,
        &plan,
        straight(&func),
    );
    assert_eq!(
        js,
        "function f($v0, $v1, $v2, $v3) {\n  let $cell = [];\n  same(__rt.box($v0.r), __rt.box($v1));\n  same(__rt.box($v2[1]), { buf: $v0, off: \"r\" });\n  $cell[0] = $v1;\n  same(__rt.box($cell[0]), { buf: $v3.buf, off: $v3.off });\n}\n"
    );
}

/// A reference-to-raw cast builds its pointer from the operand's spelling (`ptr.rs::ref_to_raw`),
/// so the planner keeps a reference loaded from a field out of it: inline, the cast would answer
/// the field's own slot.
#[test]
fn a_reference_to_raw_cast_reads_a_loaded_reference_through_a_name() {
    let mut interns = world();
    let (holder, _) = reference_holders(&mut interns);
    let same = interns.intern_str("same");
    let mut b = FuncBuilder::new(item(1), signature(vec![holder], UNIT));
    let e = FuncBuilder::ENTRY;
    let a = b.param(holder, NO_ORIGIN);
    let loaded = b.op(
        e,
        OpKind::Field {
            base: a,
            variant: None,
            field: 1,
        },
        REF_POINT,
        NO_ORIGIN,
    );
    let raw = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::PtrToPtr,
            v: loaded,
            from: REF_POINT,
            to: RAW_POINT,
        },
        RAW_POINT,
        NO_ORIGIN,
    );
    b.effect(e, call_op(same, vec![raw]), NO_ORIGIN);
    b.terminate(e, RTerm::Return(None));
    let func = b.finish();
    func.verify(&interns.types).unwrap();
    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    assert!(!plan.inline.contains(&loaded));
    assert!(plan.inline.contains(&raw));
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(
        &interns,
        "f",
        &["same"],
        &body,
        &func,
        &plan,
        straight(&func),
    );
    assert_eq!(
        js,
        "function f($v0) {\n  let $v1;\n  $v1 = $v0.r;\n  same(__rt.box($v1));\n}\n"
    );
}

/// The walk keeps a place "held as a value" ([`super::place::Walk::object`], so its raw pointer
/// is the object's box) only while the place is still the object the reference names: through a
/// downcast and through every level of `#[repr(transparent)]` wrapping, and for an aggregate value
/// with no projection at all. A step into storage ends it, and the raw pointer is then the slot of
/// that storage: a real field (even one of an aggregate type), an element, and a scalar payload of
/// an enum variant. Every root is a reference read from a field and spelled inline (`$v0.w`), the
/// spelling the slot shortcut would otherwise have read as the place.
#[test]
fn a_raw_view_boxes_a_held_object_until_a_step_reaches_storage() {
    const W: TypeId = TypeId(53);
    const W2: TypeId = TypeId(54);
    const REF_W2: TypeId = TypeId(55);
    const REF_SHAPE: TypeId = TypeId(56);
    const RAW_SHAPE: TypeId = TypeId(57);
    const HOLDS: TypeId = TypeId(58);
    const REF_HOLDS: TypeId = TypeId(59);
    const REF_POINTS: TypeId = TypeId(60);
    const REFS: TypeId = TypeId(61);
    let mut interns = world();
    let (_, points) = reference_holders(&mut interns);
    let same = interns.intern_str("same");
    let mut name = |text: &str| interns.intern_str(text);
    let (wrapper, holds_name, refs_name) = (name("W"), name("Holds"), name("Refs"));
    let (p, n, w, s, h, xs, pt) = (
        name("p"),
        name("n"),
        name("w"),
        name("s"),
        name("h"),
        name("xs"),
        name("pt"),
    );
    // `#[repr(transparent)] struct W(Point); #[repr(transparent)] struct W2(W);`
    for (id, inner, def_n) in [(W, POINT, 41), (W2, W, 42)] {
        let fields = vec![field(FieldKey::Index(0), inner)];
        let mut ty = descriptor(
            id,
            TypeKind::Adt {
                def: def(def_n),
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![variant(wrapper, fields.clone(), 0, None)],
                niche: None,
            },
            8,
        );
        ty.js.is_indirect = true;
        ty.js.needs_clone = true;
        ty.js.transparent = Some(rcgjs_object::jir::types::TransparentField {
            field: 0,
            ty: inner,
        });
        ty.js.peeled = POINT;
        ty.js.fields = fields;
        interns.intern_type(ty);
    }
    let mut record = |id: TypeId, def_n: u64, name: StrId, fields: Vec<FieldDescriptor>| {
        let mut ty = descriptor(
            id,
            TypeKind::Adt {
                def: def(def_n),
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![variant(name, fields.clone(), 0, None)],
                niche: None,
            },
            12,
        );
        ty.js.is_indirect = true;
        ty.js.needs_clone = true;
        ty.js.fields = fields;
        interns.intern_type(ty);
    };
    // `struct Holds { p: Point, n: i32 }`: a field of an aggregate type is still storage.
    record(
        HOLDS,
        43,
        holds_name,
        vec![
            field(FieldKey::Name(p), POINT),
            field(FieldKey::Name(n), I32),
        ],
    );
    // `struct Refs<'a> { w: &W2, s: &Shape, h: &Holds, xs: &[Point; 2], pt: Point }`.
    record(
        REFS,
        44,
        refs_name,
        vec![
            field(FieldKey::Name(w), REF_W2),
            field(FieldKey::Name(s), REF_SHAPE),
            field(FieldKey::Name(h), REF_HOLDS),
            field(FieldKey::Name(xs), REF_POINTS),
            field(FieldKey::Name(pt), POINT),
        ],
    );
    interns.intern_type(pointer(REF_W2, W2, PointerForm::Thin, false));
    interns.intern_type(pointer(REF_SHAPE, SHAPE, PointerForm::Thin, false));
    interns.intern_type(pointer(RAW_SHAPE, SHAPE, PointerForm::Slot, true));
    interns.intern_type(pointer(REF_HOLDS, HOLDS, PointerForm::Thin, false));
    interns.intern_type(pointer(REF_POINTS, points, PointerForm::Thin, false));

    let mut b = FuncBuilder::new(item(1), signature(vec![REFS, USIZE], UNIT));
    let e = FuncBuilder::ENTRY;
    let refs = b.param(REFS, NO_ORIGIN);
    let index = b.param(USIZE, NO_ORIGIN);
    // `&raw const <path>` of `refs.<field>`, the field read right before it so it is inline.
    let raw = |b: &mut FuncBuilder, field: u32, field_ty: TypeId, path: Vec<Proj>, ty: TypeId| {
        let loaded = b.op(
            e,
            OpKind::Field {
                base: refs,
                variant: None,
                field,
            },
            field_ty,
            NO_ORIGIN,
        );
        b.op(
            e,
            OpKind::RawView {
                root: Root::Value(loaded),
                path,
                mutable: false,
            },
            ty,
            NO_ORIGIN,
        )
    };
    let wrapped = vec![
        Proj::Deref,
        Proj::Field { field: 0, ty: W },
        Proj::Field {
            field: 0,
            ty: POINT,
        },
    ];
    let pairs = [
        // `&raw const (*refs.w).0.0` and `&raw const *(refs.s as Named)`: still the object.
        [
            (0, REF_W2, wrapped, RAW_POINT),
            (
                1,
                REF_SHAPE,
                vec![Proj::Deref, Proj::Downcast { variant: 3 }],
                RAW_SHAPE,
            ),
        ],
        // `&raw const (*refs.h).p` and `&raw const (*refs.xs)[index]`: storage.
        [
            (
                2,
                REF_HOLDS,
                vec![
                    Proj::Deref,
                    Proj::Field {
                        field: 0,
                        ty: POINT,
                    },
                ],
                RAW_POINT,
            ),
            (
                3,
                REF_POINTS,
                vec![Proj::Deref, Proj::Index(index)],
                RAW_POINT,
            ),
        ],
        // `&raw const (*refs.s as Num).0` is storage; `&raw const refs.pt` read as a value is not.
        [
            (
                1,
                REF_SHAPE,
                vec![
                    Proj::Deref,
                    Proj::Downcast { variant: 1 },
                    Proj::Field { field: 0, ty: I32 },
                ],
                RAW_I32,
            ),
            (4, POINT, Vec::new(), RAW_POINT),
        ],
    ];
    for pair in pairs {
        let args = pair
            .into_iter()
            .map(|(field, field_ty, path, ty)| raw(&mut b, field, field_ty, path, ty))
            .collect();
        b.effect(e, call_op(same, args), NO_ORIGIN);
    }
    b.terminate(e, RTerm::Return(None));

    let func = b.finish();
    func.verify(&interns.types).unwrap();
    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(
        &interns,
        "f",
        &["same"],
        &body,
        &func,
        &plan,
        straight(&func),
    );
    assert_eq!(
        js,
        "function f($v0, $v1) {\n  let $v12;\n  same(__rt.box($v0.w), __rt.box($v0.s));\n  same({ buf: $v0.h, off: \"p\" }, { buf: $v0.xs, off: $v1 });\n  $v12 = $v0.pt;\n  same({ buf: $v0.s, off: \"_0\" }, __rt.box($v12));\n}\n"
    );
}

#[test]
fn i64_pair_needed_helpers_co_locate_writers_with_shared_statics() {
    let referenced: BTreeSet<JsName> = [
        super::i64_pair::HELPER_MUL,
        super::i64_pair::HELPER_MUL_OVF,
        super::i64_pair::HELPER_DIV,
    ]
    .into_iter()
    .map(|name| JsName::new(name.to_owned()))
    .collect();
    let items = super::i64_pair::needed_helper_items(&referenced, &BTreeSet::new());
    let hi = items
        .iter()
        .find(|item| item.name.as_str() == super::i64_pair::HELPER_HI)
        .expect("HELPER_HI present");
    let hi_refs: BTreeSet<&str> = hi.refs.iter().map(|r| r.as_str()).collect();
    assert!(hi_refs.contains(super::i64_pair::HELPER_MUL));
    assert!(hi_refs.contains(super::i64_pair::HELPER_MUL_OVF));
    assert!(hi_refs.contains(super::i64_pair::HELPER_DIV));
    assert!(!hi_refs.contains(super::i64_pair::HELPER_F2I));

    let ovf = items
        .iter()
        .find(|item| item.name.as_str() == super::i64_pair::HELPER_OVF)
        .expect("HELPER_OVF present");
    let ovf_refs: BTreeSet<&str> = ovf.refs.iter().map(|r| r.as_str()).collect();
    assert!(ovf_refs.contains(super::i64_pair::HELPER_MUL_OVF));
}

#[test]
fn i64_pair_pack_8_bytes_masks_dynamic_inputs_with_255() {
    let (lo, hi) = super::i64_pair::pack_8_bytes([
        jsast::id("b0"),
        jsast::num(-1.0),
        jsast::id("b2"),
        jsast::id("b3"),
        jsast::id("b4"),
        jsast::id("b5"),
        jsast::id("b6"),
        jsast::id("b7"),
    ]);
    assert_eq!(
        jsast::expr_to_string(&lo),
        "b0 & 255 | 255 << 8 | ((b2 & 255) << 16 | b3 << 24)"
    );
    assert_eq!(
        jsast::expr_to_string(&hi),
        "b4 & 255 | (b5 & 255) << 8 | ((b6 & 255) << 16 | b7 << 24)"
    );
}

#[test]
fn flat_abi_i64_pair_param_with_param_cell_declares_box_instead_of_rebinding_missing_name() {
    let interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![I64, I64], I64));
    let x = b.param(I64, NO_ORIGIN);
    let y = b.param(I64, NO_ORIGIN);
    let cell = b.cell(I64, true, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    b.effect(e, OpKind::CellAlloc(cell), NO_ORIGIN);
    b.effect(e, OpKind::CellStore { cell, v: x }, NO_ORIGIN);
    let ptr = b.op(
        e,
        OpKind::BorrowView {
            root: Root::Cell(cell),
            path: Vec::new(),
            mutable: true,
        },
        REF_I32,
        NO_ORIGIN,
    );
    b.effect(e, OpKind::StoreThrough { ptr, v: y }, NO_ORIGIN);
    let loaded = b.op(e, OpKind::CellLoad(cell), I64, NO_ORIGIN);
    b.terminate(e, RTerm::Return(Some(loaded)));
    let func = b.finish();

    let mut plan =
        super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    plan.names.insert(x, "x".to_owned());
    plan.names.insert(y, "y".to_owned());
    plan.cell_names.insert(cell, "x".to_owned());
    plan.param_cells.insert(cell, x);

    let flat_i64 = crate::plan::abi_flat::FlatParam {
        ty: I64,
        field_tys: Vec::new(),
        passed: Vec::new(),
        by_ref: false,
        mutable: false,
        written: false,
        slice: false,
        i64_pair: true,
        tagged: None,
        recursive: None,
    };
    let target_abi = crate::plan::abi_flat::FlatSig {
        params: vec![Some(flat_i64.clone()), Some(flat_i64)],
        result: None,
        spill: 0,
        returns_out_field: false,
    };

    let body = body_of(BodyKind::Function, func.sig.clone());
    let opts = Opts::default();
    let name = JsName::new("callee_addr_taken".to_owned());
    let summary = summary(&[]);
    let spills = [JsName::new("$r1".to_owned())];
    let abi = crate::plan::abi_flat::AbiPlan::for_target_test(item(1), name.clone(), target_abi);
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
        spills: &spills,
        abi: Some(&abi),
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
        bigints: None,
    };
    let lowered = function::lower(&input, &func, &plan, straight(&func)).expect("lowers");
    let js = jsast::stmt_to_string(&lowered.decl);
    assert!(
        js.contains("function callee_addr_taken(x$lo, x$hi, y$lo, y$hi)"),
        "expected expanded signature, got:\n{js}"
    );
    assert!(
        js.contains("let x = [];") || js.contains("let x = [],"),
        "expected boxed param cell declaration for expanded param, got:\n{js}"
    );
    assert!(
        js.contains("x[0] = "),
        "expected CellStore through boxed cell `x[0] = ...`, got:\n{js}"
    );
    assert!(
        !js.contains("\n  x = ["),
        "must not reassign undeclared `x = [...]`, got:\n{js}"
    );
}

#[test]
fn i64_pair_to_f32_uses_sticky_helpers_without_double_rounding() {
    let u32_expr =
        super::i64_pair::i64_pair_to_float(false, 32, (jsast::id("lo"), jsast::id("hi")));
    let i32_expr = super::i64_pair::i64_pair_to_float(true, 32, (jsast::id("lo"), jsast::id("hi")));
    let u64_expr =
        super::i64_pair::i64_pair_to_float(false, 64, (jsast::id("lo"), jsast::id("hi")));
    assert_eq!(jsast::expr_to_string(&u32_expr), "$__mires_u64_f32(lo, hi)");
    assert_eq!(jsast::expr_to_string(&i32_expr), "$__mires_i64_f32(lo, hi)");
    assert_eq!(
        jsast::expr_to_string(&u64_expr),
        "(hi >>> 0) * 4294967296 + (lo >>> 0)"
    );
    let preamble = super::i64_pair::helper_preamble_js();
    assert!(preamble.contains("function $__mires_u64_f32(lo, hi)"));
    assert!(preamble.contains("function $__mires_i64_f32(lo, hi)"));
}

#[test]
fn i64_pair_fast_ops_mul_rotate_and_div_u32_fast_path() {
    let preamble = super::i64_pair::helper_preamble_js();
    assert!(
        preamble.contains("if (!signed && (ah | bh) === 0 && bl !== 0)"),
        "expected u32 fast path in HELPER_DIV:\n{preamble}"
    );

    let mut interns = world();
    let u64_ty = TypeId(90);
    interns.types.insert(int(u64_ty, false, 64));
    let rot_left = interns.intern_str("rotate_left");
    let rot_right = interns.intern_str("rotate_right");

    let mut b = FuncBuilder::new(item(1), signature(vec![u64_ty], u64_ty));
    let x = b.param(u64_ty, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let c5 = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 5, bytes: 8 }),
        u64_ty,
        NO_ORIGIN,
    );
    let c8 = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 8, bytes: 8 }),
        u64_ty,
        NO_ORIGIN,
    );
    let c31 = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 31, bytes: 4 }),
        U32,
        NO_ORIGIN,
    );
    let c33 = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 33, bytes: 4 }),
        U32,
        NO_ORIGIN,
    );
    let m5 = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Mul,
            l: x,
            r: c5,
            ty: u64_ty,
        },
        u64_ty,
        NO_ORIGIN,
    );
    let m8 = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Mul,
            l: m5,
            r: c8,
            ty: u64_ty,
        },
        u64_ty,
        NO_ORIGIN,
    );
    let rl = b.op(
        e,
        OpKind::Intrinsic {
            name: rot_left,
            types: vec![u64_ty],
            fallback: None,
            args: vec![m8, c31],
        },
        u64_ty,
        NO_ORIGIN,
    );
    let rr = b.op(
        e,
        OpKind::Intrinsic {
            name: rot_right,
            types: vec![u64_ty],
            fallback: None,
            args: vec![rl, c33],
        },
        u64_ty,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(rr)));
    let func = b.finish();
    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower_with(
        &interns,
        "fast_u64",
        &body,
        &func,
        &plan,
        straight(&func),
        super::host::EMPTY_ITEMS,
        &super::unsize::EMPTY_VTABLES,
    );
    assert!(
        !js.contains(super::i64_pair::HELPER_MUL),
        "small const mul must inline instead of calling HELPER_MUL:\n{js}"
    );
    assert!(
        js.contains("Math.imul("),
        "mul by 5 should inline Math.imul 16-bit split:\n{js}"
    );
    assert!(
        js.contains("<< 3"),
        "mul by 8 should lower via shift:\n{js}"
    );
    assert!(
        !js.contains("& 63"),
        "constant rotate should emit branchless Int32 shifts without & 63:\n{js}"
    );
}

#[test]
fn ptr_str_len_folding_bytes_str_fusion_and_single_eval_byte_ptr() {
    let mut interns = world();
    let raw_u8 = TypeId(91);
    interns.types.insert(descriptor(
        raw_u8,
        TypeKind::RawPointer {
            pointee: U8,
            mutable: false,
        },
        4,
    ));
    let link = interns.intern_str("next_str");
    let ptr_link = interns.intern_str("next_ptr");

    // Direct `str_len` and `str_to_bytes` simplifications.
    let hello = interns.intern_str("hello");
    let mut b0 = FuncBuilder::new(item(1), signature(Vec::new(), USIZE));
    let s_lit = b0.op(
        FuncBuilder::ENTRY,
        OpKind::Const(ConstTree::Str(hello)),
        STR_REF,
        NO_ORIGIN,
    );
    let len0 = b0.op(
        FuncBuilder::ENTRY,
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v: s_lit,
            ty: STR_REF,
        },
        USIZE,
        NO_ORIGIN,
    );
    b0.terminate(FuncBuilder::ENTRY, RTerm::Return(Some(len0)));
    let f0 = b0.finish();
    let p0 = super::super::fuse::plan(&f0, None, &interns.types, &interns, &Opts::default());
    let body0 = body_of(BodyKind::Function, f0.sig.clone());
    let js0 = lower_with(
        &interns,
        "lit_len",
        &body0,
        &f0,
        &p0,
        straight(&f0),
        super::host::EMPTY_ITEMS,
        &super::unsize::EMPTY_VTABLES,
    );
    assert!(
        js0.contains("return 5;") && !js0.contains("str_len"),
        "constant string literal length must fold to 5:\n{js0}"
    );

    // Non-trivial `&str -> *const u8` evaluates `str_bytes` once and preserves `off`.
    let mut b1 = FuncBuilder::new(item(2), signature(vec![USIZE], raw_u8));
    let n1 = b1.param(USIZE, NO_ORIGIN);
    let e1 = FuncBuilder::ENTRY;
    let call_s = b1.op(
        e1,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(3),
                link_name: link,
            },
            function: None,
            args: Vec::new(),
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        STR_REF,
        NO_ORIGIN,
    );
    let ptr_u8 = b1.op(
        e1,
        OpKind::Cast {
            kind: CastKind::PtrToPtr,
            v: call_s,
            from: STR_REF,
            to: raw_u8,
        },
        raw_u8,
        NO_ORIGIN,
    );
    let call_p = b1.op(
        e1,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(4),
                link_name: ptr_link,
            },
            function: None,
            args: vec![ptr_u8],
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        RAW_I32,
        NO_ORIGIN,
    );
    let added_p = b1.op(
        e1,
        OpKind::Binary {
            op: BinaryOp::Offset,
            l: call_p,
            r: n1,
            ty: RAW_I32,
        },
        RAW_I32,
        NO_ORIGIN,
    );
    let back_u8 = b1.op(
        e1,
        OpKind::Cast {
            kind: CastKind::PtrToPtr,
            v: added_p,
            from: RAW_I32,
            to: raw_u8,
        },
        raw_u8,
        NO_ORIGIN,
    );
    b1.terminate(e1, RTerm::Return(Some(back_u8)));
    let f1 = b1.finish();
    let p1 = super::super::fuse::plan(&f1, None, &interns.types, &interns, &Opts::default());
    let body1 = body_of(BodyKind::Function, f1.sig.clone());
    let js1 = lower_with(
        &interns,
        "str_ptr_and_offset",
        &body1,
        &f1,
        &p1,
        straight(&f1),
        super::host::EMPTY_ITEMS,
        &super::unsize::EMPTY_VTABLES,
    );
    assert_eq!(
        js1.matches("__rt.str_bytes(").count(),
        1,
        "str_bytes must be evaluated once:\n{js1}"
    );
    assert!(
        js1.contains(".off") && !js1.contains("off: 0"),
        "str_to_byte_ptr must preserve sb.off:\n{js1}"
    );
    assert_eq!(
        js1.matches("next_ptr(").count(),
        1,
        "pointer offset must evaluate non-trivial pointer operand once:\n{js1}"
    );
}

fn add_canonical_string_types(interns: &mut Interns) -> TypeId {
    use crate::jir::types::{AllocatorIdentity, RepresentationCandidate};

    let raw_u8 = TypeId(91);
    let raw_vec = TypeId(94);
    let vec_u8 = TypeId(95);
    let string_ty = TypeId(96);
    interns.types.insert(descriptor(
        raw_u8,
        TypeKind::RawPointer {
            pointee: U8,
            mutable: false,
        },
        4,
    ));
    let ptr_name = interns.intern_str("ptr");
    let cap_name = interns.intern_str("cap");
    let buf_name = interns.intern_str("buf");
    let len_name = interns.intern_str("len");
    let vec_name = interns.intern_str("vec");
    let rv_name = interns.intern_str("RawVec");
    let v_name = interns.intern_str("Vec");
    let s_name = interns.intern_str("String");
    interns.types.insert(descriptor(
        raw_vec,
        TypeKind::Adt {
            def: def(94),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                rv_name,
                vec![
                    field(FieldKey::Name(ptr_name), raw_u8),
                    field(FieldKey::Name(cap_name), USIZE),
                ],
                0,
                None,
            )],
            niche: None,
        },
        8,
    ));
    let mut vec_desc = descriptor(
        vec_u8,
        TypeKind::Adt {
            def: def(95),
            args: vec![U8],
            form: NominalForm::Struct,
            variants: vec![variant(
                v_name,
                vec![
                    field(FieldKey::Name(buf_name), raw_vec),
                    field(FieldKey::Name(len_name), USIZE),
                ],
                0,
                None,
            )],
            niche: None,
        },
        12,
    );
    vec_desc.meta.candidate = RepresentationCandidate::Vec;
    vec_desc.meta.allocator = AllocatorIdentity::Global;
    interns.types.insert(vec_desc);
    let mut str_desc = descriptor(
        string_ty,
        TypeKind::Adt {
            def: def(96),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                s_name,
                vec![field(FieldKey::Name(vec_name), vec_u8)],
                0,
                None,
            )],
            niche: None,
        },
        12,
    );
    str_desc.meta.candidate = RepresentationCandidate::String;
    str_desc.meta.allocator = AllocatorIdentity::Global;
    interns.types.insert(str_desc);
    string_ty
}

#[test]
fn fuse_local_bytes_str_eliminates_roundtrip_local() {
    use crate::opts::OptimizerMode;

    let mut interns = world();
    let slice_u8 = TypeId(92);
    interns
        .types
        .insert(descriptor(slice_u8, TypeKind::Slice(U8), 8));
    let ref_slice_u8 = TypeId(93);
    let mut ref_slice_desc = pointer(ref_slice_u8, slice_u8, PointerForm::FatSlice, false);
    ref_slice_desc.kind = TypeKind::Reference {
        pointee: slice_u8,
        mutable: false,
    };
    interns.types.insert(ref_slice_desc);

    let mut b = FuncBuilder::new(item(1), signature(vec![ref_slice_u8], USIZE));
    let raw_bytes = b.param(ref_slice_u8, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let str_val = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::PtrToPtr,
            v: raw_bytes,
            from: ref_slice_u8,
            to: STR_REF,
        },
        STR_REF,
        NO_ORIGIN,
    );
    let len = b.op(
        e,
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v: str_val,
            ty: STR_REF,
        },
        USIZE,
        NO_ORIGIN,
    );
    let back_bytes = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::PtrToPtr,
            v: str_val,
            from: STR_REF,
            to: ref_slice_u8,
        },
        ref_slice_u8,
        NO_ORIGIN,
    );
    let len2 = b.op(
        e,
        OpKind::Unary {
            op: UnaryOp::PtrMetadata,
            v: back_bytes,
            ty: ref_slice_u8,
        },
        USIZE,
        NO_ORIGIN,
    );
    let sum = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: len,
            r: len2,
            ty: USIZE,
        },
        USIZE,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(sum)));
    let func = b.finish();

    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    let body = body_of(BodyKind::Function, func.sig.clone());
    let opts = Opts {
        optimizer: OptimizerMode::On,
        ..Opts::default()
    };
    let name = JsName::new("sb_len".to_owned());
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
        bigints: None,
    };
    let lowered = function::lower(&input, &func, &plan, straight(&func)).expect("lowers");
    let js = jsast::stmt_to_string(&lowered.decl);
    assert!(
        !js.contains("__rt.bytes_str")
            && !js.contains("__rt.str_len")
            && !js.contains("__rt.str_bytes"),
        "expected fuse_local_bytes_str to eliminate bytes_str/str_len/str_bytes roundtrip:\n{js}"
    );
    assert!(
        js.contains(".len"),
        "expected direct .len read after fusion:\n{js}"
    );
}

#[test]
fn simplify_addr_zero_eq_lowers_to_ptr_is_null() {
    let mut interns = world();
    let raw_u8 = TypeId(91);
    interns.types.insert(descriptor(
        raw_u8,
        TypeKind::RawPointer {
            pointee: U8,
            mutable: false,
        },
        4,
    ));

    let mut b = FuncBuilder::new(item(1), signature(vec![raw_u8, RAW_I32], BOOL));
    let p_u8 = b.param(raw_u8, NO_ORIGIN);
    let p_i32 = b.param(RAW_I32, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let zero1 = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let zero2 = b.op(
        e,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        USIZE,
        NO_ORIGIN,
    );
    let a1 = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::PointerExposeProvenance,
            v: p_u8,
            from: raw_u8,
            to: USIZE,
        },
        USIZE,
        NO_ORIGIN,
    );
    let a4 = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::PointerExposeProvenance,
            v: p_i32,
            from: RAW_I32,
            to: USIZE,
        },
        USIZE,
        NO_ORIGIN,
    );
    let eq_null = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Eq,
            l: a1,
            r: zero1,
            ty: BOOL,
        },
        BOOL,
        NO_ORIGIN,
    );
    let ne_null = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::Ne,
            l: a4,
            r: zero2,
            ty: BOOL,
        },
        BOOL,
        NO_ORIGIN,
    );
    let both = b.op(
        e,
        OpKind::Binary {
            op: BinaryOp::BitAnd,
            l: eq_null,
            r: ne_null,
            ty: BOOL,
        },
        BOOL,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(both)));
    let func = b.finish();
    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower_with(
        &interns,
        "check_nulls",
        &body,
        &func,
        &plan,
        straight(&func),
        super::host::EMPTY_ITEMS,
        &super::unsize::EMPTY_VTABLES,
    );
    assert!(!js.contains("__rt.addr("), "{js}");
    assert!(js.contains("__rt.ptr_is_null("), "{js}");
    assert!(js.contains(", 4)"), "align=4 must be preserved: {js}");
}

#[test]
fn host_string_and_string_builder_drop_elide_baseline_conversion() {
    use crate::jir::repr::Carrier;

    let mut interns = world();
    let string_ty = add_canonical_string_types(&mut interns);
    let drop_name = interns.intern_str("drop_str");
    let mut b = FuncBuilder::new(item(1), signature(vec![string_ty, string_ty], UNIT));
    let p_host = b.param(string_ty, NO_ORIGIN);
    let p_base = b.param(string_ty, NO_ORIGIN);
    let e = FuncBuilder::ENTRY;
    let host_to_base = b.op(
        e,
        OpKind::Convert {
            v: p_host,
            from: Carrier::HostString,
            to: Carrier::Baseline,
        },
        string_ty,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::Drop {
            root: Root::Value(host_to_base),
            path: Vec::new(),
            glue: DropGlue::Direct {
                item: item(2),
                link_name: drop_name,
            },
        },
        NO_ORIGIN,
    );
    let base_to_sb = b.op(
        e,
        OpKind::Convert {
            v: p_base,
            from: Carrier::Baseline,
            to: Carrier::StringBuilder,
        },
        string_ty,
        NO_ORIGIN,
    );
    b.effect(
        e,
        OpKind::Drop {
            root: Root::Value(base_to_sb),
            path: Vec::new(),
            glue: DropGlue::Direct {
                item: item(2),
                link_name: drop_name,
            },
        },
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(None));

    let mut func = b.finish();
    func.values[p_host.0 as usize].carrier = Carrier::HostString;
    func.values[p_base.0 as usize].carrier = Carrier::Baseline;
    func.values[host_to_base.0 as usize].carrier = Carrier::Baseline;
    func.values[base_to_sb.0 as usize].carrier = Carrier::StringBuilder;

    let plan = super::super::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower_with(
        &interns,
        "drop_strings",
        &body,
        &func,
        &plan,
        straight(&func),
        super::host::EMPTY_ITEMS,
        &super::unsize::EMPTY_VTABLES,
    );
    assert!(!js.contains("convert_str_block"), "{js}");
    assert!(!js.contains("bytes_str"), "{js}");
    assert!(js.contains("__rt.dealloc("), "{js}");
}

#[test]
fn fuse_local_bytes_str_rejects_aliased_mutations_non_dominance_and_inner_fn() {
    use rcgjs_object::jsast::{ArrowBody, BinOp, Expr, Stmt};
    use std::collections::HashSet;

    let bytes_str_call = |buf: &str| {
        Expr::Call(
            Box::new(Expr::Member(
                Box::new(Expr::Ident("__rt".to_owned())),
                "bytes_str".to_owned(),
            )),
            vec![Expr::Ident(buf.to_owned()), Expr::Num(0.0), Expr::Num(4.0)],
        )
    };
    let str_call = |method: &str, var: &str| {
        Expr::Call(
            Box::new(Expr::Member(
                Box::new(Expr::Ident("__rt".to_owned())),
                method.to_owned(),
            )),
            vec![Expr::Ident(var.to_owned())],
        )
    };

    // 1. Buffer mutation between `bytes_str` and `str_bytes` must prevent lazy slice reuse.
    let mut mut_body = vec![
        Stmt::ExprStmt(Expr::Assign(
            Box::new(Expr::Ident("x".to_owned())),
            Box::new(bytes_str_call("b")),
        )),
        Stmt::ExprStmt(Expr::Assign(
            Box::new(Expr::Index(
                Box::new(Expr::Ident("b".to_owned())),
                Box::new(Expr::Num(0.0)),
            )),
            Box::new(Expr::Num(90.0)),
        )),
        Stmt::Return(Some(str_call("str_bytes", "x"))),
    ];
    let locals: HashSet<String> = ["x".to_owned()].into_iter().collect();
    function::fuse_local_bytes_str(&mut mut_body, &locals);
    let mut_js = mut_body
        .iter()
        .map(jsast::stmt_to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        mut_js.contains("__rt.bytes_str(") && mut_js.contains("__rt.str_bytes(x)"),
        "mutation before str_bytes must prevent fusion:\n{mut_js}"
    );

    // 2. Call between `bytes_str` and `str_bytes` must prevent lazy slice reuse,
    //    whereas a call between `bytes_str` and pure `str_len` is safe.
    let mut call_before_bytes = vec![
        Stmt::ExprStmt(Expr::Assign(
            Box::new(Expr::Ident("x".to_owned())),
            Box::new(bytes_str_call("b")),
        )),
        Stmt::ExprStmt(Expr::Call(
            Box::new(Expr::Ident("mutate_b".to_owned())),
            Vec::new(),
        )),
        Stmt::Return(Some(str_call("str_bytes", "x"))),
    ];
    function::fuse_local_bytes_str(&mut call_before_bytes, &locals);
    let call_bytes_js = call_before_bytes
        .iter()
        .map(jsast::stmt_to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        call_bytes_js.contains("__rt.bytes_str(") && call_bytes_js.contains("__rt.str_bytes(x)"),
        "call before str_bytes must prevent fusion:\n{call_bytes_js}"
    );

    let mut call_before_len = vec![
        Stmt::ExprStmt(Expr::Assign(
            Box::new(Expr::Ident("x".to_owned())),
            Box::new(bytes_str_call("b")),
        )),
        Stmt::ExprStmt(Expr::Call(
            Box::new(Expr::Ident("mutate_b".to_owned())),
            Vec::new(),
        )),
        Stmt::Return(Some(str_call("str_len", "x"))),
    ];
    function::fuse_local_bytes_str(&mut call_before_len, &locals);
    let call_len_js = call_before_len
        .iter()
        .map(jsast::stmt_to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !call_len_js.contains("__rt.bytes_str") && call_len_js.contains("x.len"),
        "call before str_len should still fuse scalar len:\n{call_len_js}"
    );

    // 3. Non-dominating conditional write must prevent fusion.
    let mut cond_body = vec![
        Stmt::If(
            Expr::Ident("c".to_owned()),
            vec![Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Ident("x".to_owned())),
                Box::new(bytes_str_call("b")),
            ))],
            None,
        ),
        Stmt::Return(Some(str_call("str_len", "x"))),
    ];
    function::fuse_local_bytes_str(&mut cond_body, &locals);
    let cond_js = cond_body
        .iter()
        .map(jsast::stmt_to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        cond_js.contains("__rt.bytes_str(") && cond_js.contains("__rt.str_len(x)"),
        "non-dominating write must prevent fusion:\n{cond_js}"
    );

    // 4. Nested FunctionDecl or Arrow capturing `x` must disqualify `x`.
    let mut fn_decl_body = vec![
        Stmt::FunctionDecl {
            name: "inner".to_owned(),
            params: Vec::new(),
            body: vec![Stmt::Return(Some(str_call("str_len", "x")))],
        },
        Stmt::ExprStmt(Expr::Assign(
            Box::new(Expr::Ident("x".to_owned())),
            Box::new(bytes_str_call("b")),
        )),
        Stmt::Return(Some(Expr::Binary(
            BinOp::Add,
            Box::new(str_call("str_len", "x")),
            Box::new(Expr::Call(
                Box::new(Expr::Ident("inner".to_owned())),
                Vec::new(),
            )),
        ))),
    ];
    function::fuse_local_bytes_str(&mut fn_decl_body, &locals);
    let fn_decl_js = fn_decl_body
        .iter()
        .map(jsast::stmt_to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        fn_decl_js.contains("__rt.bytes_str(") && fn_decl_js.contains("__rt.str_len(x)"),
        "nested FunctionDecl reading outer local must prevent fusion:\n{fn_decl_js}"
    );

    // Unused closure referencing ArrowBody::Expr to silence unused import when no Arrow test above.
    let _ = ArrowBody::Expr(Box::new(Expr::Undefined));
}

#[test]
fn host_string_inline_call_drop_preserves_call_and_non_adjacent_baseline_dealloc_stays() {
    use rcgjs_object::jir::repr::Carrier;
    use rcgjs_object::opts::OptimizerMode;

    let mut interns = world();
    let string_ty = add_canonical_string_types(&mut interns);
    let impure_name = interns.intern_str("impure_host_str");
    let poke_name = interns.intern_str("poke_str");
    let drop_name = interns.intern_str("drop_str");

    // Case 1: `let _ = impure_host_str();` where Convert -> Drop inlines the HostString call.
    let mut b1 = FuncBuilder::new(item(1), signature(Vec::new(), UNIT));
    let e1 = FuncBuilder::ENTRY;
    let host_call = b1.op(
        e1,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(2),
                link_name: impure_name,
            },
            function: None,
            args: Vec::new(),
            semantic: CallSemantic::Ordinary,
            host: None,
            track_caller: None,
        },
        string_ty,
        NO_ORIGIN,
    );
    let host_to_base = b1.op(
        e1,
        OpKind::Convert {
            v: host_call,
            from: Carrier::HostString,
            to: Carrier::Baseline,
        },
        string_ty,
        NO_ORIGIN,
    );
    b1.effect(
        e1,
        OpKind::Drop {
            root: Root::Value(host_to_base),
            path: Vec::new(),
            glue: DropGlue::Direct {
                item: item(3),
                link_name: drop_name,
            },
        },
        NO_ORIGIN,
    );
    b1.terminate(e1, RTerm::Return(None));
    let mut func1 = b1.finish();
    func1.values[host_call.0 as usize].carrier = Carrier::HostString;
    func1.values[host_to_base.0 as usize].carrier = Carrier::Baseline;

    let opts = Opts {
        optimizer: OptimizerMode::On,
        ..Opts::default()
    };
    let plan1 = super::super::fuse::plan(&func1, None, &interns.types, &interns, &opts);
    let body1 = body_of(BodyKind::Function, func1.sig.clone());
    let input1 = LowerInput {
        name: &JsName::new("keep_impure_host_str".to_owned()),
        summary: &summary(&[]),
        body: &body1,
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
        bigints: None,
    };
    let js1 = jsast::stmt_to_string(
        &function::lower(&input1, &func1, &plan1, straight(&func1))
            .expect("lowers")
            .decl,
    );
    assert!(
        js1.contains("impure_host_str()"),
        "impure call side effect must survive drop elision:\n{js1}"
    );
    assert!(!js1.contains("convert_str_block"), "{js1}");
    assert!(!js1.contains("drop_str"), "{js1}");

    // Case 2: non-adjacent BaselineDealloc (intervening call between Convert and Drop) must NOT
    // defer `src.vec` reads past the intervening call.
    let indirect_str_ty = TypeId(97);
    let mut indirect_str = interns
        .types
        .get(string_ty)
        .expect("string descriptor")
        .clone();
    indirect_str.id = indirect_str_ty;
    indirect_str.js.is_indirect = true;
    indirect_str.js.peeled = indirect_str_ty;
    assert!(interns.types.insert(indirect_str));

    let mut b2 = FuncBuilder::new(item(4), signature(vec![indirect_str_ty], UNIT));
    let p_base = b2.param(indirect_str_ty, NO_ORIGIN);
    let e2 = FuncBuilder::ENTRY;
    let base_to_sb = b2.op(
        e2,
        OpKind::Convert {
            v: p_base,
            from: Carrier::Baseline,
            to: Carrier::StringBuilder,
        },
        indirect_str_ty,
        NO_ORIGIN,
    );
    let _poke = b2.op(
        e2,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(5),
                link_name: poke_name,
            },
            function: None,
            args: Vec::new(),
            semantic: CallSemantic::Ordinary,
            host: None,
            track_caller: None,
        },
        UNIT,
        NO_ORIGIN,
    );
    b2.effect(
        e2,
        OpKind::Drop {
            root: Root::Value(base_to_sb),
            path: Vec::new(),
            glue: DropGlue::Direct {
                item: item(3),
                link_name: drop_name,
            },
        },
        NO_ORIGIN,
    );
    b2.terminate(e2, RTerm::Return(None));
    let mut func2 = b2.finish();
    func2.values[p_base.0 as usize].carrier = Carrier::Baseline;
    func2.values[base_to_sb.0 as usize].carrier = Carrier::StringBuilder;

    let plan2 = super::super::fuse::plan(&func2, None, &interns.types, &interns, &opts);
    let body2 = body_of(BodyKind::Function, func2.sig.clone());
    let input2 = LowerInput {
        name: &JsName::new("non_adjacent_sb_drop".to_owned()),
        summary: &summary(&[]),
        body: &body2,
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
        bigints: None,
    };
    let js2 = jsast::stmt_to_string(
        &function::lower(&input2, &func2, &plan2, straight(&func2))
            .expect("lowers")
            .decl,
    );
    assert!(
        !js2.contains("__rt.dealloc(")
            && js2.contains("__rt.bytes_str(")
            && js2.contains("drop_str("),
        "non-adjacent BaselineDealloc must not defer src snapshot past intervening call:\n{js2}"
    );
}

#[test]
fn caller_location_value_disambiguates_core_location_from_user_location_struct() {
    use rcgjs_object::jir::{Origin, OriginId, OriginKind, OriginTable, SourceFact};

    let mut interns = world();
    let loc_name = interns.intern_str("Location");
    let name_key = interns.intern_str("name");
    let x_key = interns.intern_str("x");
    let y_key = interns.intern_str("y");

    let user_loc_ty = TypeId(9901);
    let user_loc_ref_ty = TypeId(9902);
    let mut user_loc = descriptor(
        user_loc_ty,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 1,
                local_hash: 999,
            },
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                loc_name,
                vec![
                    field(FieldKey::Name(name_key), STR_REF),
                    field(FieldKey::Name(x_key), U32),
                    field(FieldKey::Name(y_key), U32),
                ],
                0,
                None,
            )],
            niche: None,
        },
        12,
    );
    user_loc.js.is_indirect = true;
    user_loc.js.needs_clone = true;
    interns.intern_type(user_loc);
    interns.intern_type(pointer(user_loc_ref_ty, user_loc_ty, PointerForm::Thin, false));

    // Re-register `LOCATION` under a distinct crate id (`99`) so both `Intrinsic("caller_location")`
    // and non-intrinsic `track_caller` calls resolve `core`'s `(file, line, column)`.
    let file_key = interns.intern_str("file");
    let line_key = interns.intern_str("line");
    let col_key = interns.intern_str("column");
    let mut core_loc = descriptor(
        LOCATION,
        TypeKind::Adt {
            def: DefinitionId {
                crate_id: 99,
                local_hash: 16,
            },
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                loc_name,
                vec![
                    field(FieldKey::Name(file_key), STR_REF),
                    field(FieldKey::Name(line_key), U32),
                    field(FieldKey::Name(col_key), U32),
                ],
                0,
                None,
            )],
            niche: None,
        },
        12,
    );
    core_loc.js.is_indirect = true;
    core_loc.js.needs_clone = true;
    interns.types.replace(core_loc);

    let file_id = interns.intern_str("probe.rs");
    let intr_name = interns.intern_str("caller_location");
    let link = interns.intern_str("tc_callee$h2");
    let item_path = interns.intern_str("test::fn");
    let fact = interns.intern_source_fact(SourceFact {
        file: file_id,
        line: 12,
        col: 3,
    });
    let origins = OriginTable {
        origins: vec![Origin {
            kind: OriginKind::Statement,
            source: Some(fact),
            local_name: None,
            inline_stack: Vec::new(),
        }],
        statements: Vec::new(),
        terminators: Vec::new(),
        locals: Vec::new(),
        item_path,
        root: None,
    };

    // 1) Direct `caller_location` intrinsic alongside `&user::Location` parameter.
    let e = FuncBuilder::ENTRY;
    let mut b1 = FuncBuilder::new(item(1), signature(vec![user_loc_ref_ty], REF_LOCATION));
    let _u = b1.param(user_loc_ref_ty, NO_ORIGIN);
    let call_intr = b1.op(
        e,
        OpKind::Intrinsic {
            name: intr_name,
            types: Vec::new(),
            fallback: None,
            args: Vec::new(),
        },
        REF_LOCATION,
        OriginId(0),
    );
    b1.terminate(e, RTerm::Return(Some(call_intr)));
    let f1 = b1.finish();
    let p1 = plan_of(&f1, &[(0, "u")]);
    let body1 = body_of(BodyKind::Function, f1.sig.clone());
    let mut input1 = LowerInput {
        name: &JsName::new("direct_intr".to_owned()),
        summary: &summary(&[]),
        body: &body1,
        origins: Some(&origins),
        interns: &interns,
        table: &interns.types,
        opts: &Opts::default(),
        items: super::host::EMPTY_ITEMS,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
        bigints: None,
    };
    let js1 = jsast::stmt_to_string(
        &function::lower(&input1, &f1, &p1, straight(&f1))
            .expect("direct caller_location lowers")
            .decl,
    );
    assert!(
        js1.contains("file:") && js1.contains("line:") && js1.contains("column:") && !js1.contains("name:"),
        "{js1}"
    );

    // 2) Call to another `#[track_caller]` fn without `caller_location` intrinsic in caller body.
    let mut b2 = FuncBuilder::new(item(1), signature(vec![user_loc_ref_ty], I32));
    let _u2 = b2.param(user_loc_ref_ty, NO_ORIGIN);
    let call_tc = b2.op(
        e,
        OpKind::Call {
            callee: Callee::Direct {
                item: item(2),
                link_name: link,
            },
            function: None,
            args: Vec::new(),
            track_caller: Some(fact),
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        OriginId(0),
    );
    b2.terminate(e, RTerm::Return(Some(call_tc)));
    let f2 = b2.finish();
    let p2 = plan_of(&f2, &[(0, "u")]);
    let mut items = super::host::ItemIndex::new();
    items.insert(
        item(2),
        super::host::ItemEntry {
            abi_known: true,
            name: "tc_callee$h2".to_owned(),
            params: 1,
            result: Some(I32),
            arguments: Vec::new(),
            caller_location: true,
            caller_location_def: None,
            is_core: false,
            used_params: BTreeSet::new(),
            standard: None,
            keyed_purity: None,
            keyed_read_params: None,
            keyed_may_unwind: None,
            keyed_row_capture: None,
            keyed_proxy_safe_params: None,
        },
    );
    input1.items = &items;
    let body2 = body_of(BodyKind::Function, f2.sig.clone());
    input1.body = &body2;
    let js2 = jsast::stmt_to_string(
        &function::lower(&input1, &f2, &p2, straight(&f2))
            .expect("forwarding track_caller call lowers")
            .decl,
    );
    assert!(
        js2.contains("file:") && js2.contains("line:") && js2.contains("column:") && !js2.contains("name:"),
        "{js2}"
    );

    // 3) Third-crate (`alloc`) caller whose crate-scope TypeTable mentions `user::Location`,
    //    while `core` panic item in `ItemIndex` carries `is_core: true`.
    let alloc_item = ItemId {
        definition: DefinitionId {
            crate_id: 77,
            local_hash: 500,
        },
        instantiation: 0,
    };
    let core_panic_item = ItemId {
        definition: DefinitionId {
            crate_id: 99,
            local_hash: 501,
        },
        instantiation: 0,
    };
    let mut b3 = FuncBuilder::new(alloc_item, signature(vec![], I32));
    let call_panic = b3.op(
        e,
        OpKind::Call {
            callee: Callee::Direct {
                item: core_panic_item,
                link_name: link,
            },
            function: None,
            args: Vec::new(),
            track_caller: Some(fact),
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        OriginId(0),
    );
    b3.terminate(e, RTerm::Return(Some(call_panic)));
    let f3 = b3.finish();
    let p3 = plan_of(&f3, &[]);
    let mut items3 = super::host::ItemIndex::new();
    items3.insert(
        core_panic_item,
        super::host::ItemEntry {
            abi_known: true,
            name: "tc_callee$h2".to_owned(),
            params: 1,
            result: Some(I32),
            arguments: Vec::new(),
            caller_location: true,
            caller_location_def: None,
            is_core: true,
            used_params: BTreeSet::new(),
            standard: None,
            keyed_purity: None,
            keyed_read_params: None,
            keyed_may_unwind: None,
            keyed_row_capture: None,
            keyed_proxy_safe_params: None,
        },
    );
    let body3 = body_of(BodyKind::Function, f3.sig.clone());
    let input3 = LowerInput {
        name: &JsName::new("alloc_caller".to_owned()),
        summary: &summary(&[]),
        body: &body3,
        origins: Some(&origins),
        interns: &interns,
        table: &interns.types,
        opts: &Opts::default(),
        items: &items3,
        drop_glue: super::host::EMPTY_DROP_GLUE,
        vtables: &super::unsize::EMPTY_VTABLES,
        spills: &[],
        abi: None,
        physical: None,
        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
        bigints: None,
    };
    let js3 = jsast::stmt_to_string(
        &function::lower(&input3, &f3, &p3, straight(&f3))
            .expect("cross-crate alloc caller resolves core Location via is_core")
            .decl,
    );
    assert!(
        js3.contains("file:") && js3.contains("line:") && js3.contains("column:") && !js3.contains("name:"),
        "{js3}"
    );
}

#[test]
fn signed_niche_from_scalar_compares_signed_literal() {
    let mut interns = world();
    let i8_ty = TypeId(901);
    let ordering = TypeId(902);
    let opt_ordering = TypeId(903);
    interns.intern_type(int(i8_ty, true, 8));

    let less = interns.intern_str("Less");
    let equal = interns.intern_str("Equal");
    let greater = interns.intern_str("Greater");
    let mut ord_desc = descriptor(
        ordering,
        TypeKind::Adt {
            def: def(902),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(less, Vec::new(), -1, Some(255)),
                variant(equal, Vec::new(), 0, Some(0)),
                variant(greater, Vec::new(), 1, Some(1)),
            ],
            niche: None,
        },
        1,
    );
    ord_desc.js.enum_repr = Some(EnumRepr::Number {
        bits: 8,
        signed: true,
    });
    ord_desc.js.is_primitive_enum = true;
    interns.intern_type(ord_desc);

    let none = interns.intern_str("None");
    let some = interns.intern_str("Some");
    let f0 = interns.intern_str("_0");
    let mut opt_ord = descriptor(
        opt_ordering,
        TypeKind::Adt {
            def: def(903),
            args: vec![ordering],
            form: NominalForm::Enum,
            variants: vec![
                variant(none, Vec::new(), 0, None),
                variant(some, vec![field(FieldKey::Name(f0), ordering)], 1, None),
            ],
            niche: Some(rcgjs_object::jir::types::Niche {
                untagged_variant: 1,
                niche_field: vec![0],
                niche_start: 254,
                valid_range: (255, 1),
            }),
        },
        1,
    );
    opt_ord.js.is_indirect = true;
    opt_ord.js.needs_clone = true;
    opt_ord.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(opt_ord);

    // i8 -> Option<Ordering> compares against -2 (NOT 254!)
    let mut b = FuncBuilder::new(item(901), signature(vec![i8_ty], opt_ordering));
    let e = FuncBuilder::ENTRY;
    let p = b.param(i8_ty, NO_ORIGIN);
    let cast = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::Transmute,
            v: p,
            from: i8_ty,
            to: opt_ordering,
        },
        opt_ordering,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(cast)));
    let func = b.finish();
    let plan = plan_of(&func, &[(p.0, "x")]);
    let body = body_of(BodyKind::Function, func.sig.clone());
    let js = lower(
        &interns,
        "dec_i8",
        &[],
        &body,
        &func,
        &plan,
        straight(&func),
    );
    assert!(js.contains("$n === -2"), "{js}");
    assert!(!js.contains("$n === 254"), "{js}");

    // u8 -> Option<Ordering> compares against 254 and sign-extends payload to i8
    let mut b2 = FuncBuilder::new(item(902), signature(vec![U8], opt_ordering));
    let p2 = b2.param(U8, NO_ORIGIN);
    let cast2 = b2.op(
        e,
        OpKind::Cast {
            kind: CastKind::Transmute,
            v: p2,
            from: U8,
            to: opt_ordering,
        },
        opt_ordering,
        NO_ORIGIN,
    );
    b2.terminate(e, RTerm::Return(Some(cast2)));
    let func2 = b2.finish();
    let plan2 = plan_of(&func2, &[(p2.0, "x")]);
    let body2 = body_of(BodyKind::Function, func2.sig.clone());
    let js2 = lower(
        &interns,
        "dec_u8",
        &[],
        &body2,
        &func2,
        &plan2,
        straight(&func2),
    );
    assert!(js2.contains("$n === 254"), "{js2}");
    assert!(js2.contains("<< 24"), "{js2}");
}

#[test]
fn wide_u128_high_bit_niche_roundtrips_without_sign_flip() {
    let mut interns = world();
    let u128_ty = TypeId(910);
    let i128_ty = TypeId(911);
    let hi_enum = TypeId(912);
    let opt_hi = TypeId(913);
    interns.intern_type(int(u128_ty, false, 128));
    interns.intern_type(int(i128_ty, true, 128));

    let a_name = interns.intern_str("A");
    let b_name = interns.intern_str("B");
    let mut hi_desc = descriptor(
        hi_enum,
        TypeKind::Adt {
            def: def(912),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(a_name, Vec::new(), 0, Some(0)),
                variant(b_name, Vec::new(), 1, Some(1)),
            ],
            niche: None,
        },
        16,
    );
    hi_desc.js.enum_repr = Some(EnumRepr::Number {
        bits: 128,
        signed: false,
    });
    hi_desc.js.is_primitive_enum = true;
    interns.intern_type(hi_desc);

    let none = interns.intern_str("None");
    let some = interns.intern_str("Some");
    let f0 = interns.intern_str("_0");
    let mut opt_desc = descriptor(
        opt_hi,
        TypeKind::Adt {
            def: def(913),
            args: vec![hi_enum],
            form: NominalForm::Enum,
            variants: vec![
                variant(none, Vec::new(), 0, None),
                variant(some, vec![field(FieldKey::Name(f0), hi_enum)], 1, None),
            ],
            niche: Some(rcgjs_object::jir::types::Niche {
                untagged_variant: 1,
                niche_field: vec![0],
                niche_start: u128::MAX,
                valid_range: (0, 1),
            }),
        },
        16,
    );
    opt_desc.js.is_indirect = true;
    opt_desc.js.needs_clone = true;
    opt_desc.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(opt_desc);

    let e = FuncBuilder::ENTRY;
    // u128 -> opt_hi must compare against 340282366920938463463374607431768211455n
    let mut bd = FuncBuilder::new(item(910), signature(vec![u128_ty], opt_hi));
    let pd = bd.param(u128_ty, NO_ORIGIN);
    let cd = bd.op(
        e,
        OpKind::Cast {
            kind: CastKind::Transmute,
            v: pd,
            from: u128_ty,
            to: opt_hi,
        },
        opt_hi,
        NO_ORIGIN,
    );
    bd.terminate(e, RTerm::Return(Some(cd)));
    let fd = bd.finish();
    let js_dec = lower(
        &interns,
        "dec_u128",
        &[],
        &body_of(BodyKind::Function, fd.sig.clone()),
        &fd,
        &plan_of(&fd, &[(pd.0, "x")]),
        straight(&fd),
    );
    assert!(
        js_dec.contains("340282366920938463463374607431768211455n"),
        "{js_dec}"
    );
    assert!(!js_dec.contains("-1n"), "{js_dec}");

    // opt_hi -> u128 must encode None as 340282366920938463463374607431768211455n (NOT -1n!)
    let mut be = FuncBuilder::new(item(911), signature(vec![opt_hi], u128_ty));
    let pe = be.param(opt_hi, NO_ORIGIN);
    let ce = be.op(
        e,
        OpKind::Cast {
            kind: CastKind::Transmute,
            v: pe,
            from: opt_hi,
            to: u128_ty,
        },
        u128_ty,
        NO_ORIGIN,
    );
    be.terminate(e, RTerm::Return(Some(ce)));
    let fe = be.finish();
    let js_enc = lower(
        &interns,
        "enc_u128",
        &[],
        &body_of(BodyKind::Function, fe.sig.clone()),
        &fe,
        &plan_of(&fe, &[(pe.0, "x")]),
        straight(&fe),
    );
    assert!(
        js_enc.contains("340282366920938463463374607431768211455n"),
        "{js_enc}"
    );
    assert!(!js_enc.contains("-1n"), "{js_enc}");
}

#[test]
fn multi_wrapping_niche_and_transparent_width_decode_accurately() {
    let mut interns = world();
    let i8_ty = TypeId(920);
    let wrap_ty = TypeId(921);
    let wrap_enum = TypeId(922);
    interns.intern_type(int(i8_ty, true, 8));

    let f0 = interns.intern_str("_0");
    let wrap_name = interns.intern_str("Wrap");
    let pat_u8 = TypeId(923);
    let mut pat_desc = descriptor(pat_u8, TypeKind::Pattern(U8), 1);
    pat_desc.js.peeled = U8;
    interns.intern_type(pat_desc);
    let mut wrap_desc = descriptor(
        wrap_ty,
        TypeKind::Adt {
            def: def(921),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant(
                wrap_name,
                vec![field(FieldKey::Transparent, pat_u8)],
                0,
                None,
            )],
            niche: None,
        },
        1,
    );
    wrap_desc.js.peeled = U8;
    interns.intern_type(wrap_desc);

    let p_name = interns.intern_str("P");
    let n0_name = interns.intern_str("N0");
    let n1_name = interns.intern_str("N1");
    let n2_name = interns.intern_str("N2");
    let niche = rcgjs_object::jir::types::Niche {
        untagged_variant: 0,
        niche_field: vec![0, 0, 0],
        niche_start: 254,
        valid_range: (1, 253),
    };
    let variants = vec![
        variant(p_name, vec![field(FieldKey::Name(f0), wrap_ty)], 0, None),
        variant(n0_name, Vec::new(), 1, None),
        variant(n1_name, Vec::new(), 2, None),
        variant(n2_name, Vec::new(), 3, None),
    ];
    let mut enum_desc = descriptor(
        wrap_enum,
        TypeKind::Adt {
            def: def(922),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: variants.clone(),
            niche: Some(niche.clone()),
        },
        1,
    );
    enum_desc.js.is_indirect = true;
    enum_desc.js.needs_clone = true;
    enum_desc.js.enum_repr = Some(EnumRepr::Tag { fieldless: false });
    interns.intern_type(enum_desc);

    // niche_width must walk unpeeled transparent wrapper [0, 0] and return Some(8)
    assert_eq!(
        super::value::niche_width(&interns.types, &variants, &niche),
        Some(8)
    );
    // bits = 0 wraps: (0 - 254) & 0xFF == 2 -> variant 3 (N2)
    assert_eq!(
        super::value::niche_variant(&interns.types, &variants, &niche, 0),
        3
    );

    // i8 -> wrap_enum checks -2, -1, 0
    let e = FuncBuilder::ENTRY;
    let mut b = FuncBuilder::new(item(920), signature(vec![i8_ty], wrap_enum));
    let p = b.param(i8_ty, NO_ORIGIN);
    let c = b.op(
        e,
        OpKind::Cast {
            kind: CastKind::Transmute,
            v: p,
            from: i8_ty,
            to: wrap_enum,
        },
        wrap_enum,
        NO_ORIGIN,
    );
    b.terminate(e, RTerm::Return(Some(c)));
    let f = b.finish();
    let js = lower(
        &interns,
        "dec_wrap_i8",
        &[],
        &body_of(BodyKind::Function, f.sig.clone()),
        &f,
        &plan_of(&f, &[(p.0, "x")]),
        straight(&f),
    );
    assert!(js.contains("$n === -2"), "{js}");
    assert!(js.contains("$n === -1"), "{js}");
    assert!(js.contains("$n === 0"), "{js}");
}

#[test]
fn niche_width_selects_unique_inhabited_enum_variant_and_never_returns_zero() {
    let mut interns = world();
    let infallible_ty = TypeId(930);
    let ordering_ty = TypeId(931);
    let single_live_ord = TypeId(932);
    let single_live_char = TypeId(933);
    let opt_ord_ty = TypeId(934);
    let single_live_opt = TypeId(935);

    interns.intern_type(descriptor(
        infallible_ty,
        TypeKind::Adt {
            def: def(930),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: Vec::new(),
            niche: None,
        },
        0,
    ));

    let less = interns.intern_str("Less");
    let equal = interns.intern_str("Equal");
    let greater = interns.intern_str("Greater");
    let mut ord_desc = descriptor(
        ordering_ty,
        TypeKind::Adt {
            def: def(931),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(less, Vec::new(), -1, Some(255)),
                variant(equal, Vec::new(), 0, Some(0)),
                variant(greater, Vec::new(), 1, Some(1)),
            ],
            niche: None,
        },
        1,
    );
    ord_desc.js.enum_repr = Some(EnumRepr::Number {
        bits: 8,
        signed: true,
    });
    ord_desc.js.is_primitive_enum = true;
    interns.intern_type(ord_desc);

    let dead = interns.intern_str("Dead");
    let live = interns.intern_str("Live");
    let none = interns.intern_str("None");
    let some = interns.intern_str("Some");
    let f0 = interns.intern_str("_0");
    let f1 = interns.intern_str("_1");

    // enum SingleLive { Dead(Infallible), Live(Ordering) }
    interns.intern_type(descriptor(
        single_live_ord,
        TypeKind::Adt {
            def: def(932),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(
                    dead,
                    vec![field(FieldKey::Name(f0), infallible_ty)],
                    0,
                    None,
                ),
                variant(live, vec![field(FieldKey::Name(f0), ordering_ty)], 1, None),
            ],
            niche: None,
        },
        1,
    ));

    // enum SingleLiveChar { Dead(u8, Infallible), Live(char) }
    interns.intern_type(descriptor(
        single_live_char,
        TypeKind::Adt {
            def: def(933),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(
                    dead,
                    vec![
                        field(FieldKey::Name(f0), U8),
                        field(FieldKey::Name(f1), infallible_ty),
                    ],
                    0,
                    None,
                ),
                variant(live, vec![field(FieldKey::Name(f0), CHAR)], 1, None),
            ],
            niche: None,
        },
        4,
    ));

    // Option<Ordering>
    interns.intern_type(descriptor(
        opt_ord_ty,
        TypeKind::Adt {
            def: def(934),
            args: vec![ordering_ty],
            form: NominalForm::Enum,
            variants: vec![
                variant(none, Vec::new(), 0, None),
                variant(some, vec![field(FieldKey::Name(f0), ordering_ty)], 1, None),
            ],
            niche: Some(rcgjs_object::jir::types::Niche {
                untagged_variant: 1,
                niche_field: vec![0],
                niche_start: 254,
                valid_range: (255, 1),
            }),
        },
        1,
    ));

    // enum SingleLiveOpt { Dead(Infallible), Live(Option<Ordering>) }
    interns.intern_type(descriptor(
        single_live_opt,
        TypeKind::Adt {
            def: def(935),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                variant(
                    dead,
                    vec![field(FieldKey::Name(f0), infallible_ty)],
                    0,
                    None,
                ),
                variant(live, vec![field(FieldKey::Name(f0), opt_ord_ty)], 1, None),
            ],
            niche: None,
        },
        1,
    ));

    // Option<SingleLive>: niche_field = [0, 0], niche_start = 254 -> width = 8, bits = 255 -> Some (1)
    let opt_sl_variants = vec![
        variant(none, Vec::new(), 0, None),
        variant(
            some,
            vec![field(FieldKey::Name(f0), single_live_ord)],
            1,
            None,
        ),
    ];
    let opt_sl_niche = rcgjs_object::jir::types::Niche {
        untagged_variant: 1,
        niche_field: vec![0, 0],
        niche_start: 254,
        valid_range: (255, 1),
    };
    assert_eq!(
        super::value::niche_width(&interns.types, &opt_sl_variants, &opt_sl_niche),
        Some(8)
    );
    assert_eq!(
        super::value::niche_variant(&interns.types, &opt_sl_variants, &opt_sl_niche, 254),
        0
    );
    assert_eq!(
        super::value::niche_variant(&interns.types, &opt_sl_variants, &opt_sl_niche, 255),
        1
    );

    // Option<SingleLiveChar>: niche_field = [0, 0] must step to char (32), NOT Dead's u8 (8)!
    let opt_slc_variants = vec![
        variant(none, Vec::new(), 0, None),
        variant(
            some,
            vec![field(FieldKey::Name(f0), single_live_char)],
            1,
            None,
        ),
    ];
    let opt_slc_niche = rcgjs_object::jir::types::Niche {
        untagged_variant: 1,
        niche_field: vec![0, 0],
        niche_start: u32::MAX as u128,
        valid_range: (0, 0x10_FFFF),
    };
    assert_eq!(
        super::value::niche_width(&interns.types, &opt_slc_variants, &opt_slc_niche),
        Some(32)
    );
    assert_eq!(
        super::value::niche_variant(&interns.types, &opt_slc_variants, &opt_slc_niche, 0xFF),
        1
    );

    // Option<SingleLiveOpt>: niche_field = [0, 0] recurses through inner Option<Ordering> -> Some(8)
    let opt_slo_variants = vec![
        variant(none, Vec::new(), 0, None),
        variant(
            some,
            vec![field(FieldKey::Name(f0), single_live_opt)],
            1,
            None,
        ),
    ];
    let opt_slo_niche = rcgjs_object::jir::types::Niche {
        untagged_variant: 1,
        niche_field: vec![0, 0],
        niche_start: 253,
        valid_range: (254, 1),
    };
    assert_eq!(
        super::value::niche_width(&interns.types, &opt_slo_variants, &opt_slo_niche),
        Some(8)
    );
    assert_eq!(
        super::value::niche_variant(&interns.types, &opt_slo_variants, &opt_slo_niche, 254),
        1
    );

    // A 0-byte leaf must return None rather than Some(0)
    let zero_variants = vec![
        variant(none, Vec::new(), 0, None),
        variant(
            some,
            vec![field(FieldKey::Name(f0), infallible_ty)],
            1,
            None,
        ),
    ];
    let zero_niche = rcgjs_object::jir::types::Niche {
        untagged_variant: 1,
        niche_field: vec![0],
        niche_start: 0,
        valid_range: (1, 1),
    };
    assert_eq!(
        super::value::niche_width(&interns.types, &zero_variants, &zero_niche),
        None
    );
}

mod invoke;
