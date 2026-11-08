//! Unit tests: every supported operation lowered at every legal carrier against the pinned v1
//! spelling, the registry table, and the string and vec `Convert` adapters over a
//! hand-built canonical `String`/`Vec` type tree.

use crate::item::JsName;
use crate::jir::interns::Interns;
use crate::jir::repr::Carrier;
use crate::jir::semantic::{AbiSignature, BodySemantic, Signature, StandardOperation};
use crate::jir::types::{
    AllocatorIdentity, FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts,
    NominalDiscriminant, NominalForm, RepresentationCandidate, TypeDescriptor, TypeId, TypeKind,
    TypeMeta, VariantDescriptor,
};
use crate::jsast;

use super::*;

const UNIT: TypeId = TypeId(1);
const U8: TypeId = TypeId(2);
const USIZE: TypeId = TypeId(3);
const BOOL: TypeId = TypeId(4);
const CHAR: TypeId = TypeId(5);
const I32: TypeId = TypeId(6);
const F64: TypeId = TypeId(7);
const STR: TypeId = TypeId(8);
const STR_REF: TypeId = TypeId(9);
const STRING: TypeId = TypeId(10);
const STRING_REF: TypeId = TypeId(11);
const STRING_MUT: TypeId = TypeId(12);
const VEC_U8: TypeId = TypeId(13);
const RAW_VEC: TypeId = TypeId(14);
const RAW_VEC_INNER: TypeId = TypeId(15);
const UNIQUE_U8: TypeId = TypeId(16);
const CAP: TypeId = TypeId(17);
const RAW_U8: TypeId = TypeId(18);
const ALLOC_ZST: TypeId = TypeId(19);
const MARKER_ZST: TypeId = TypeId(20);
const FMT_ERROR: TypeId = TypeId(21);
const FMT_RESULT: TypeId = TypeId(22);
const ARGUMENTS: TypeId = TypeId(23);
const ARGUMENT: TypeId = TypeId(24);
const F64_REF: TypeId = TypeId(25);
const I32_REF: TypeId = TypeId(26);
const BOOL_REF: TypeId = TypeId(27);
const CHAR_REF: TypeId = TypeId(28);
const PIECES_REF: TypeId = TypeId(29);
const ARGS_REF: TypeId = TypeId(30);
const U32: TypeId = TypeId(31);
const VEC_U32: TypeId = TypeId(32);
const VEC_U32_REF: TypeId = TypeId(33);
const VEC_U32_MUT: TypeId = TypeId(34);
const SLICE_U32: TypeId = TypeId(35);
const SLICE_REF: TypeId = TypeId(36);
const SLICE_MUT: TypeId = TypeId(37);
const U32_REF: TypeId = TypeId(38);
const U32_MUT: TypeId = TypeId(39);
const DROPPY: TypeId = TypeId(40);
const VEC_DROPPY: TypeId = TypeId(41);
const VEC_DROPPY_MUT: TypeId = TypeId(42);
const OWN_DROP: TypeId = TypeId(43);
const VEC_OWN_DROP: TypeId = TypeId(44);
const VEC_OWN_DROP_MUT: TypeId = TypeId(45);
const VEC_ZST: TypeId = TypeId(46);
const HOLDER: TypeId = TypeId(47);
const BOX_U32: TypeId = TypeId(48);
const BOX_U32_MUT: TypeId = TypeId(49);
const BOX_DROPPY: TypeId = TypeId(50);
const BOX_DROPPY_MUT: TypeId = TypeId(51);
const BOX_OWN_DROP: TypeId = TypeId(52);
const BOX_OWN_DROP_MUT: TypeId = TypeId(53);
const VEC_DROPPY_REF: TypeId = TypeId(54);
const DROPPY_REF: TypeId = TypeId(55);

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
            is_indirect: false,
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
        repr: IntRepr::Number,
    });
    d.js.byte_element = bits == 8 && !signed;
    d
}

fn reference(id: TypeId, pointee: TypeId, mutable: bool) -> TypeDescriptor {
    descriptor(
        id,
        TypeKind::Reference { pointee, mutable },
        4, // The producer's 32-bit pointer target: what the capacity limits derive from.
    )
}

fn def_id(n: u64) -> crate::jir::semantic::DefinitionId {
    crate::jir::semantic::DefinitionId {
        crate_id: 1,
        local_hash: n,
    }
}

fn field(interns: &mut Interns, name: &str, ty: TypeId) -> FieldDescriptor {
    FieldDescriptor {
        key: FieldKey::Name(interns.intern_str(name)),
        ty,
        offset: 0,
    }
}

fn strukt(
    interns: &mut Interns,
    id: TypeId,
    name: &str,
    fields: Vec<FieldDescriptor>,
    size: u64,
) -> TypeDescriptor {
    let variant = VariantDescriptor {
        name: interns.intern_str(name),
        fields: fields.clone(),
        logical_discr: 0,
        physical_tag: None,
        result_ok: false,
        saved_local_keys: Vec::new(),
    };
    let mut d = descriptor(
        id,
        TypeKind::Adt {
            def: def_id(id.0 as u64),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![variant],
            niche: None,
        },
        size,
    );
    d.js.is_indirect = true;
    d.js.fields = fields;
    d
}

/// The canonical type tree the string cluster and the adapters read: the primitives, `&str`,
/// the baseline `String { vec: Vec<u8> { buf: RawVec { inner: { ptr, cap, alloc }, _marker },
/// len } }`, and the `Result<(), fmt::Error>` of the `fmt::Write` sinks.
fn table() -> (TypeTable, Interns) {
    let mut interns = Interns::new();
    let mut table = TypeTable::new();
    let put = |d: TypeDescriptor, table: &mut TypeTable| assert!(table.insert(d));

    put(descriptor(UNIT, TypeKind::Tuple(Vec::new()), 0), &mut table);
    put(int(U8, false, 8), &mut table);
    put(int(USIZE, false, 32), &mut table);
    put(descriptor(BOOL, TypeKind::Bool, 1), &mut table);
    put(descriptor(CHAR, TypeKind::Char, 4), &mut table);
    put(int(I32, true, 32), &mut table);
    put(descriptor(F64, TypeKind::Float { bits: 64 }, 8), &mut table);
    let mut str_desc = descriptor(STR, TypeKind::Str, 0);
    str_desc.layout.sized = false;
    str_desc.layout.size = None;
    str_desc.layout.zst = false;
    put(str_desc, &mut table);
    put(reference(STR_REF, STR, false), &mut table);
    put(reference(STRING_REF, STRING, false), &mut table);
    put(reference(STRING_MUT, STRING, true), &mut table);
    put(reference(F64_REF, F64, false), &mut table);
    put(reference(I32_REF, I32, false), &mut table);
    put(reference(BOOL_REF, BOOL, false), &mut table);
    put(reference(CHAR_REF, CHAR, false), &mut table);
    put(reference(PIECES_REF, STR_REF, false), &mut table);
    put(reference(ARGS_REF, ARGUMENT, false), &mut table);

    // The raw-parts chain, transparent wrappers peeled the way the producer records them.
    put(
        descriptor(
            RAW_U8,
            TypeKind::RawPointer {
                pointee: U8,
                mutable: false,
            },
            4,
        ),
        &mut table,
    );
    let mut unique = strukt(&mut interns, UNIQUE_U8, "Unique", Vec::new(), 4);
    unique.js.peeled = RAW_U8;
    put(unique, &mut table);
    let mut cap = strukt(&mut interns, CAP, "Cap", Vec::new(), 4);
    cap.js.peeled = USIZE;
    put(cap, &mut table);
    let alloc = strukt(&mut interns, ALLOC_ZST, "Global", Vec::new(), 0);
    put(alloc, &mut table);
    let marker = strukt(&mut interns, MARKER_ZST, "PhantomData", Vec::new(), 0);
    put(marker, &mut table);

    let inner_fields = vec![
        field(&mut interns, "ptr", UNIQUE_U8),
        field(&mut interns, "cap", CAP),
        field(&mut interns, "alloc", ALLOC_ZST),
    ];
    put(
        strukt(&mut interns, RAW_VEC_INNER, "RawVecInner", inner_fields, 8),
        &mut table,
    );
    let raw_vec_fields = vec![
        field(&mut interns, "inner", RAW_VEC_INNER),
        field(&mut interns, "_marker", MARKER_ZST),
    ];
    put(
        strukt(&mut interns, RAW_VEC, "RawVec", raw_vec_fields, 8),
        &mut table,
    );
    let vec_fields = vec![
        field(&mut interns, "buf", RAW_VEC),
        field(&mut interns, "len", USIZE),
    ];
    let mut vec_u8 = strukt(&mut interns, VEC_U8, "Vec", vec_fields, 12);
    vec_u8.kind = match vec_u8.kind {
        TypeKind::Adt {
            def,
            form,
            variants,
            niche,
            ..
        } => TypeKind::Adt {
            def,
            args: vec![U8, ALLOC_ZST],
            form,
            variants,
            niche,
        },
        _ => unreachable!(),
    };
    vec_u8.meta.candidate = RepresentationCandidate::Vec;
    vec_u8.meta.allocator = AllocatorIdentity::Global;
    put(vec_u8, &mut table);
    let string_fields = vec![field(&mut interns, "vec", VEC_U8)];
    let mut string = strukt(&mut interns, STRING, "String", string_fields, 12);
    string.meta.candidate = RepresentationCandidate::String;
    string.meta.allocator = AllocatorIdentity::Global;
    put(string, &mut table);

    // `Result<(), fmt::Error>` with its language-item `Ok` marking and direct one-byte tag.
    let fmt_error = strukt(&mut interns, FMT_ERROR, "Error", Vec::new(), 0);
    put(fmt_error, &mut table);
    let ok = VariantDescriptor {
        name: interns.intern_str("Ok"),
        fields: vec![FieldDescriptor {
            key: FieldKey::Index(0),
            ty: UNIT,
            offset: 0,
        }],
        logical_discr: 0,
        physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
        result_ok: true,
        saved_local_keys: Vec::new(),
    };
    let err = VariantDescriptor {
        name: interns.intern_str("Err"),
        fields: vec![FieldDescriptor {
            key: FieldKey::Index(0),
            ty: FMT_ERROR,
            offset: 0,
        }],
        logical_discr: 1,
        physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
        result_ok: false,
        saved_local_keys: Vec::new(),
    };
    let mut result = descriptor(
        FMT_RESULT,
        TypeKind::Adt {
            def: def_id(FMT_RESULT.0 as u64),
            args: vec![UNIT, FMT_ERROR],
            form: NominalForm::Enum,
            variants: vec![ok, err],
            niche: None,
        },
        1,
    );
    result.js.is_indirect = true;
    put(result, &mut table);

    // Opaque-enough stand-ins for `fmt::Arguments` and `fmt::rt::Argument`.
    put(
        strukt(&mut interns, ARGUMENTS, "Arguments", Vec::new(), 12),
        &mut table,
    );
    put(
        strukt(&mut interns, ARGUMENT, "Argument", Vec::new(), 8),
        &mut table,
    );

    // The vec cluster: `Vec<u32>` over the same raw-parts chain, its references, its slices, and
    // two drop-shaped elements — one structural (needs drop, no destructor of its own) and one
    // with a custom destructor.
    put(int(U32, false, 32), &mut table);
    let vec_of = |interns: &mut Interns, id: TypeId, name: &str, element: TypeId| {
        let fields = vec![field(interns, "buf", RAW_VEC), field(interns, "len", USIZE)];
        let mut vector = strukt(interns, id, name, fields, 12);
        vector.kind = match vector.kind {
            TypeKind::Adt {
                def,
                form,
                variants,
                niche,
                ..
            } => TypeKind::Adt {
                def,
                args: vec![element, ALLOC_ZST],
                form,
                variants,
                niche,
            },
            _ => unreachable!(),
        };
        vector.meta.candidate = RepresentationCandidate::Vec;
        vector.meta.allocator = AllocatorIdentity::Global;
        vector
    };
    put(vec_of(&mut interns, VEC_U32, "Vec", U32), &mut table);
    put(reference(VEC_U32_REF, VEC_U32, false), &mut table);
    put(reference(VEC_U32_MUT, VEC_U32, true), &mut table);
    let mut slice = descriptor(SLICE_U32, TypeKind::Slice(U32), 0);
    slice.layout.sized = false;
    slice.layout.size = None;
    slice.layout.zst = false;
    put(slice, &mut table);
    put(reference(SLICE_REF, SLICE_U32, false), &mut table);
    put(reference(SLICE_MUT, SLICE_U32, true), &mut table);
    put(reference(U32_REF, U32, false), &mut table);
    put(reference(U32_MUT, U32, true), &mut table);

    let mut droppy = strukt(&mut interns, DROPPY, "Droppy", Vec::new(), 12);
    droppy.layout.needs_drop = true;
    put(droppy, &mut table);
    let mut vec_droppy = vec_of(&mut interns, VEC_DROPPY, "Vec", DROPPY);
    vec_droppy.layout.needs_drop = true;
    put(vec_droppy, &mut table);
    put(reference(VEC_DROPPY_REF, VEC_DROPPY, false), &mut table);
    put(reference(VEC_DROPPY_MUT, VEC_DROPPY, true), &mut table);
    put(reference(DROPPY_REF, DROPPY, false), &mut table);
    let mut own_drop = strukt(&mut interns, OWN_DROP, "OwnDrop", Vec::new(), 12);
    own_drop.layout.needs_drop = true;
    own_drop.layout.has_own_drop = true;
    put(own_drop, &mut table);
    put(
        vec_of(&mut interns, VEC_OWN_DROP, "Vec", OWN_DROP),
        &mut table,
    );
    put(reference(VEC_OWN_DROP_MUT, VEC_OWN_DROP, true), &mut table);
    put(vec_of(&mut interns, VEC_ZST, "Vec", MARKER_ZST), &mut table);

    let box_of = |interns: &mut Interns, id: TypeId, pointee: TypeId| {
        let mut boxed = strukt(interns, id, "Box", Vec::new(), 4);
        boxed.kind = match boxed.kind {
            TypeKind::Adt {
                def,
                form,
                variants,
                niche,
                ..
            } => TypeKind::Adt {
                def,
                args: vec![pointee, ALLOC_ZST],
                form,
                variants,
                niche,
            },
            _ => unreachable!(),
        };
        boxed.layout.needs_drop = true;
        boxed.layout.has_own_drop = true;
        boxed.meta.candidate = RepresentationCandidate::Box;
        boxed.meta.allocator = AllocatorIdentity::Global;
        boxed
    };
    put(box_of(&mut interns, BOX_U32, U32), &mut table);
    put(reference(BOX_U32_MUT, BOX_U32, true), &mut table);
    put(box_of(&mut interns, BOX_DROPPY, DROPPY), &mut table);
    put(reference(BOX_DROPPY_MUT, BOX_DROPPY, true), &mut table);
    put(box_of(&mut interns, BOX_OWN_DROP, OWN_DROP), &mut table);
    put(reference(BOX_OWN_DROP_MUT, BOX_OWN_DROP, true), &mut table);
    let holder_fields = vec![
        field(&mut interns, "first", VEC_DROPPY),
        field(&mut interns, "second", DROPPY),
    ];
    let mut holder = strukt(&mut interns, HOLDER, "Holder", holder_fields, 24);
    holder.layout.needs_drop = true;
    put(holder, &mut table);

    (table, interns)
}

fn sig(arguments: Vec<TypeId>, result: TypeId) -> Signature {
    let parameters = u16::try_from(arguments.len()).unwrap();
    Signature {
        arguments,
        result,
        abi: AbiSignature {
            parameters,
            spread: None,
            caller_location: false,
        },
    }
}

/// A bounds operation's signature: a trailing track-caller `Location` parameter in the
/// JavaScript ABI that never appears among the Rust arguments.
fn sig_tracked(arguments: Vec<TypeId>, result: TypeId) -> Signature {
    let parameters = u16::try_from(arguments.len()).unwrap() + 1;
    Signature {
        arguments,
        result,
        abi: AbiSignature {
            parameters,
            spread: None,
            caller_location: true,
        },
    }
}

/// The supported operations' signatures, exactly as the producer records them for the canonical
/// items.
fn signature_of(operation: StandardOperation) -> Signature {
    match operation {
        StandardOperation::StringNew => sig(vec![], STRING),
        StandardOperation::StringWithCapacity => sig(vec![USIZE], STRING),
        StandardOperation::StringLen | StandardOperation::StringCapacity => {
            sig(vec![STRING_REF], USIZE)
        }
        StandardOperation::StringPush => sig(vec![STRING_MUT, CHAR], UNIT),
        StandardOperation::StringPushStr => sig(vec![STRING_MUT, STR_REF], UNIT),
        StandardOperation::StringClear | StandardOperation::StringDrop => {
            sig(vec![STRING_MUT], UNIT)
        }
        StandardOperation::StringAsStr => sig(vec![STRING_REF], STR_REF),
        StandardOperation::StringFmtWriteStr => sig(vec![STRING_MUT, STR_REF], FMT_RESULT),
        StandardOperation::StringFmtWriteChar => sig(vec![STRING_MUT, CHAR], FMT_RESULT),
        StandardOperation::F64SpecToString => sig(vec![F64_REF], STRING),
        StandardOperation::IntToString => sig(vec![I32_REF], STRING),
        StandardOperation::BoolToString => sig(vec![BOOL_REF], STRING),
        StandardOperation::CharToString => sig(vec![CHAR_REF], STRING),
        StandardOperation::FormatArgumentNewDisplay => sig(vec![I32_REF], ARGUMENT),
        StandardOperation::FormatArgsNew => sig(vec![PIECES_REF, ARGS_REF], ARGUMENTS),
        StandardOperation::FormatArgsToString => sig(vec![ARGUMENTS], STRING),
        StandardOperation::VecNew => sig(vec![], VEC_U32),
        StandardOperation::VecWithCapacity => sig(vec![USIZE], VEC_U32),
        StandardOperation::VecLen | StandardOperation::VecCapacity => sig(vec![VEC_U32_REF], USIZE),
        StandardOperation::VecIsEmpty => sig(vec![VEC_U32_REF], BOOL),
        StandardOperation::VecPush => sig(vec![VEC_U32_MUT, U32], UNIT),
        StandardOperation::VecClear | StandardOperation::VecDrop => sig(vec![VEC_U32_MUT], UNIT),
        StandardOperation::VecAsSlice => sig(vec![VEC_U32_REF], SLICE_REF),
        StandardOperation::VecAsMutSlice => sig(vec![VEC_U32_MUT], SLICE_MUT),
        StandardOperation::VecIndex => sig_tracked(vec![VEC_U32_REF, USIZE], U32_REF),
        StandardOperation::VecIndexMut => sig_tracked(vec![VEC_U32_MUT, USIZE], U32_MUT),
        StandardOperation::VecRemove => sig_tracked(vec![VEC_U32_MUT, USIZE], U32),
        StandardOperation::VecExtend => sig(vec![VEC_U32_MUT, VEC_U32], UNIT),
        StandardOperation::SliceSwap => sig_tracked(vec![SLICE_MUT, USIZE, USIZE], UNIT),
        _ => panic!("not a supported operation: {operation:?}"),
    }
}

/// `StringCapacity`'s receiver is the mutable carrier's; everything else reads `signature_of`.
fn lower_cx<'a>(
    operation: StandardOperation,
    sig: &'a Signature,
    table: &'a TypeTable,
    interns: &'a Interns,
) -> LowerCx<'a> {
    LowerCx {
        semantic: BodySemantic::Standard(operation),
        sig,
        table,
        interns,
        element_glue: None,
    }
}

fn rendered(operation: StandardOperation, carrier: Option<Carrier>) -> String {
    let (table, interns) = table();
    let signature = signature_of(operation);
    let cx = lower_cx(operation, &signature, &table, &interns);
    // One carrier per slot: `VecExtend` has two vec slots, `SliceSwap` has none.
    let slots = registry(operation).expect("a registry entry").slots.len();
    let selected: Vec<Carrier> = carrier.into_iter().cycle().take(slots).collect();
    let rewrite = rewrite(&cx, &selected)
        .unwrap_or_else(|decline| panic!("{operation:?} at {carrier:?}: {}", decline.reason()));
    jsast::stmt_to_string(&jsast::function(
        "f",
        rewrite.parameters,
        rewrite.statements,
    ))
}

/// Every emitter at every legal carrier prints exactly the pinned v1 spelling. The pins are the
/// differential lever: `native-*-test.sh` receipts diff these helper spellings byte for byte, and
/// the capacity/bounds guard preceding every mutation in the pinned text IS the panic-timing
/// contract (D5).
#[test]
fn every_lowering_matches_its_pinned_spelling() {
    let mut pinned = PINNED.iter();
    for (operation, carriers) in SUPPORTED {
        for carrier in *carriers {
            let (expected_op, expected_carrier, expected) =
                pinned.next().expect("a pin per lowering");
            assert_eq!((expected_op, expected_carrier), (operation, carrier));
            assert_eq!(
                rendered(*operation, *carrier).trim_end(),
                *expected,
                "{operation:?} at {carrier:?} drifted from the pinned v1 spelling"
            );
        }
    }
    assert!(pinned.next().is_none(), "unclaimed pins");
}

/// Every supported lowering at every legal carrier, pinned to the exact v1 spelling.
const PINNED: &[(StandardOperation, Option<Carrier>, &str)] = &[
    (
        StandardOperation::StringNew,
        HOST,
        r#"function f() {
  return "";
}"#,
    ),
    (
        StandardOperation::StringNew,
        BUILDER,
        r#"function f() {
  return { text: "", cap: 0 };
}"#,
    ),
    (
        StandardOperation::StringWithCapacity,
        BUILDER,
        r#"function f($native0) {
  if ($native0 > 2147483647) {
    throw new RangeError("rustc_codegen_js: native String capacity overflow");
  }
  return { text: "", cap: $native0 };
}"#,
    ),
    (
        StandardOperation::StringLen,
        HOST,
        r#"function f($native0) {
  return __rt.str_len($native0);
}"#,
    ),
    (
        StandardOperation::StringLen,
        BUILDER,
        r#"function f($native0) {
  return __rt.str_len($native0.text);
}"#,
    ),
    (
        StandardOperation::StringCapacity,
        BUILDER,
        r#"function f($native0) {
  return $native0.cap;
}"#,
    ),
    (
        StandardOperation::StringPush,
        BUILDER,
        r#"function f($native0, $native1) {
  const $nativeRequired = __rt.str_len($native0.text) + __rt._utf8_width($native1);
  if ($nativeRequired > 2147483647) {
    throw new RangeError("rustc_codegen_js: native String capacity overflow");
  }
  if ($native0.cap < $nativeRequired) {
    $native0.cap = $nativeRequired;
  }
  __rt.native_str_push($native0, $native1);
  return;
}"#,
    ),
    (
        StandardOperation::StringPushStr,
        BUILDER,
        r#"function f($native0, $native1) {
  const $nativeRequired = __rt.str_len($native0.text) + __rt.str_len($native1);
  if ($nativeRequired > 2147483647) {
    throw new RangeError("rustc_codegen_js: native String capacity overflow");
  }
  if ($native0.cap < $nativeRequired) {
    $native0.cap = $nativeRequired;
  }
  $native0.text += $native1;
  return;
}"#,
    ),
    (
        StandardOperation::StringClear,
        BUILDER,
        r#"function f($native0) {
  $native0.text = "";
  return;
}"#,
    ),
    (
        StandardOperation::StringAsStr,
        HOST,
        r#"function f($native0) {
  return $native0;
}"#,
    ),
    (
        StandardOperation::StringAsStr,
        BUILDER,
        r#"function f($native0) {
  return $native0.text;
}"#,
    ),
    (
        StandardOperation::StringDrop,
        BUILDER,
        r#"function f($native0) {
  $native0.text = "";
  $native0.cap = 0;
  return;
}"#,
    ),
    (
        StandardOperation::StringFmtWriteStr,
        BUILDER,
        r#"function f($native0, $native1) {
  const $nativeRequired = __rt.str_len($native0.text) + __rt.str_len($native1);
  if ($nativeRequired > 2147483647) {
    throw new RangeError("rustc_codegen_js: native String capacity overflow");
  }
  if ($native0.cap < $nativeRequired) {
    $native0.cap = $nativeRequired;
  }
  $native0.text += $native1;
  return { TAG: "Ok", _0: undefined };
}"#,
    ),
    (
        StandardOperation::StringFmtWriteChar,
        BUILDER,
        r#"function f($native0, $native1) {
  const $nativeRequired = __rt.str_len($native0.text) + __rt._utf8_width($native1);
  if ($nativeRequired > 2147483647) {
    throw new RangeError("rustc_codegen_js: native String capacity overflow");
  }
  if ($native0.cap < $nativeRequired) {
    $native0.cap = $nativeRequired;
  }
  __rt.native_str_push($native0, $native1);
  return { TAG: "Ok", _0: undefined };
}"#,
    ),
    (
        StandardOperation::F64SpecToString,
        BUILDER,
        r#"function f($native0) {
  return __rt.native_f64_to_string($native0.buf[$native0.off]);
}"#,
    ),
    (
        StandardOperation::IntToString,
        HOST,
        r#"function f($native0) {
  return String($native0.buf[$native0.off]);
}"#,
    ),
    (
        StandardOperation::IntToString,
        BUILDER,
        r#"function f($native0) {
  const $nativeText = String($native0.buf[$native0.off]);
  return { text: $nativeText, cap: $nativeText.length };
}"#,
    ),
    (
        StandardOperation::BoolToString,
        HOST,
        r#"function f($native0) {
  return $native0.buf[$native0.off] ? "true" : "false";
}"#,
    ),
    (
        StandardOperation::BoolToString,
        BUILDER,
        r#"function f($native0) {
  const $nativeText = $native0.buf[$native0.off] ? "true" : "false";
  return { text: $nativeText, cap: $nativeText.length };
}"#,
    ),
    (
        StandardOperation::CharToString,
        HOST,
        r#"function f($native0) {
  return String.fromCodePoint($native0.buf[$native0.off]);
}"#,
    ),
    (
        StandardOperation::CharToString,
        BUILDER,
        r#"function f($native0) {
  const $nativeText = String.fromCodePoint($native0.buf[$native0.off]);
  return { text: $nativeText, cap: __rt.str_len($nativeText) };
}"#,
    ),
    (
        StandardOperation::FormatArgumentNewDisplay,
        None,
        r#"function f($native0) {
  return String($native0.buf[$native0.off]);
}"#,
    ),
    (
        StandardOperation::FormatArgsNew,
        None,
        r#"function f($native0, $native1) {
  return "";
}"#,
    ),
    (
        StandardOperation::FormatArgsToString,
        HOST,
        r#"function f($native0) {
  return $native0;
}"#,
    ),
    (
        StandardOperation::FormatArgsToString,
        BUILDER,
        r#"function f($native0) {
  const $nativeText = $native0;
  return { text: $nativeText, cap: __rt.str_len($nativeText) };
}"#,
    ),
    (
        StandardOperation::VecNew,
        NATIVE,
        r#"function f() {
  return __rt.native_vec_new(0);
}"#,
    ),
    (
        StandardOperation::VecWithCapacity,
        NATIVE,
        r#"function f($native0) {
  if ($native0 > 536870911) {
    throw new RangeError("rustc_codegen_js: native Vec capacity overflow");
  }
  return __rt.native_vec_new($native0);
}"#,
    ),
    (
        StandardOperation::VecLen,
        NATIVE,
        r#"function f($native0) {
  return $native0.len;
}"#,
    ),
    (
        StandardOperation::VecCapacity,
        NATIVE,
        r#"function f($native0) {
  return $native0.cap;
}"#,
    ),
    (
        StandardOperation::VecIsEmpty,
        NATIVE,
        r#"function f($native0) {
  return $native0.len === 0;
}"#,
    ),
    (
        StandardOperation::VecPush,
        NATIVE,
        r#"function f($native0, $native1) {
  const $nativeRequired = $native0.len + 1;
  if ($nativeRequired > 536870911) {
    throw new RangeError("rustc_codegen_js: native Vec capacity overflow");
  }
  if ($native0.len === $native0.cap) {
    __rt.native_vec_set_capacity($native0, $nativeRequired);
  }
  $native0.buf[$native0.len] = $native1;
  $native0.len += 1;
  return;
}"#,
    ),
    (
        StandardOperation::VecClear,
        NATIVE,
        r#"function f($native0) {
  __rt.native_vec_set_len($native0, 0);
  return;
}"#,
    ),
    (
        StandardOperation::VecAsSlice,
        NATIVE,
        r#"function f($native0) {
  return { buf: $native0.buf, off: 0, len: $native0.len };
}"#,
    ),
    (
        StandardOperation::VecAsMutSlice,
        NATIVE,
        r#"function f($native0) {
  return { buf: $native0.buf, off: 0, len: $native0.len };
}"#,
    ),
    (
        StandardOperation::VecIndex,
        NATIVE,
        r#"function f($native0, $native1, $native2) {
  if ($native1 >= $native0.len) {
    throw new RangeError("rustc_codegen_js: native Vec index out of bounds");
  }
  return { buf: $native0.buf, off: $native1 };
}"#,
    ),
    (
        StandardOperation::VecIndexMut,
        NATIVE,
        r#"function f($native0, $native1, $native2) {
  if ($native1 >= $native0.len) {
    throw new RangeError("rustc_codegen_js: native Vec index out of bounds");
  }
  return { buf: $native0.buf, off: $native1 };
}"#,
    ),
    (
        StandardOperation::VecRemove,
        NATIVE,
        r#"function f($native0, $native1, $native2) {
  if ($native1 >= $native0.len) {
    throw new RangeError("rustc_codegen_js: native Vec remove index out of bounds");
  }
  const $nativeRemoved = $native0.buf.splice($native1, 1)[0];
  $native0.buf.length = $native0.cap;
  $native0.len -= 1;
  return $nativeRemoved;
}"#,
    ),
    (
        StandardOperation::VecExtend,
        NATIVE,
        r#"function f($native0, $native1) {
  const $nativeTargetLen = $native0.len;
  const $nativeSourceLen = $native1.len;
  const $nativeRequired = $nativeTargetLen + $nativeSourceLen;
  if ($nativeRequired > 536870911) {
    throw new RangeError("rustc_codegen_js: native Vec capacity overflow");
  }
  if ($native0.cap < $nativeRequired) {
    __rt.native_vec_set_capacity($native0, $nativeRequired);
  }
  $native1.len = 0;
  let $nativeIndex = 0;
  while ($nativeIndex < $nativeSourceLen) {
    $native0.buf[$nativeTargetLen + $nativeIndex] = $native1.buf[$nativeIndex];
    $native1.buf[$nativeIndex] = undefined;
    $nativeIndex += 1;
  }
  $native0.len = $nativeRequired;
  __rt.native_vec_set_capacity($native1, 0);
  return;
}"#,
    ),
    (
        StandardOperation::VecDrop,
        NATIVE,
        r#"function f($native0) {
  __rt.native_vec_set_len($native0, 0);
  __rt.native_vec_set_capacity($native0, 0);
  return;
}"#,
    ),
    (
        StandardOperation::SliceSwap,
        None,
        r#"function f($native0, $native1, $native2, $native3) {
  if ($native1 >= $native0.len) {
    throw new RangeError("rustc_codegen_js: native slice swap index out of bounds");
  }
  if ($native2 >= $native0.len) {
    throw new RangeError("rustc_codegen_js: native slice swap index out of bounds");
  }
  const $nativeLeft = $native0.off + $native1;
  const $nativeRight = $native0.off + $native2;
  const $nativeValue = $native0.buf[$nativeLeft];
  $native0.buf[$nativeLeft] = $native0.buf[$nativeRight];
  $native0.buf[$nativeRight] = $nativeValue;
  return;
}"#,
    ),
];

const HOST: Option<Carrier> = Some(Carrier::HostString);
const BUILDER: Option<Carrier> = Some(Carrier::StringBuilder);
const NATIVE: Option<Carrier> = Some(Carrier::NativeVec);

const SUPPORTED: &[(StandardOperation, &[Option<Carrier>])] = &[
    (StandardOperation::StringNew, &[HOST, BUILDER]),
    (StandardOperation::StringWithCapacity, &[BUILDER]),
    (StandardOperation::StringLen, &[HOST, BUILDER]),
    (StandardOperation::StringCapacity, &[BUILDER]),
    (StandardOperation::StringPush, &[BUILDER]),
    (StandardOperation::StringPushStr, &[BUILDER]),
    (StandardOperation::StringClear, &[BUILDER]),
    (StandardOperation::StringAsStr, &[HOST, BUILDER]),
    (StandardOperation::StringDrop, &[BUILDER]),
    (StandardOperation::StringFmtWriteStr, &[BUILDER]),
    (StandardOperation::StringFmtWriteChar, &[BUILDER]),
    (StandardOperation::F64SpecToString, &[BUILDER]),
    (StandardOperation::IntToString, &[HOST, BUILDER]),
    (StandardOperation::BoolToString, &[HOST, BUILDER]),
    (StandardOperation::CharToString, &[HOST, BUILDER]),
    (StandardOperation::FormatArgumentNewDisplay, &[None]),
    (StandardOperation::FormatArgsNew, &[None]),
    (StandardOperation::FormatArgsToString, &[HOST, BUILDER]),
    (StandardOperation::VecNew, &[NATIVE]),
    (StandardOperation::VecWithCapacity, &[NATIVE]),
    (StandardOperation::VecLen, &[NATIVE]),
    (StandardOperation::VecCapacity, &[NATIVE]),
    (StandardOperation::VecIsEmpty, &[NATIVE]),
    (StandardOperation::VecPush, &[NATIVE]),
    (StandardOperation::VecClear, &[NATIVE]),
    (StandardOperation::VecAsSlice, &[NATIVE]),
    (StandardOperation::VecAsMutSlice, &[NATIVE]),
    (StandardOperation::VecIndex, &[NATIVE]),
    (StandardOperation::VecIndexMut, &[NATIVE]),
    (StandardOperation::VecRemove, &[NATIVE]),
    (StandardOperation::VecExtend, &[NATIVE]),
    (StandardOperation::VecDrop, &[NATIVE]),
    (StandardOperation::SliceSwap, &[None]),
];

/// Registry <-> `StandardOperation` is 1:1: the 33 v1-supported ops and v2's Box drop lower, while
/// the 5 operations v1 never supported remain exact declines. Every variant is spelled out so a
/// producer-side addition fails this test until the registry answers for it.
#[test]
fn registry_covers_exactly_the_34_supported_operations() {
    use StandardOperation::*;
    let lowered = [
        StringNew,
        StringWithCapacity,
        StringLen,
        StringCapacity,
        StringPush,
        StringPushStr,
        StringClear,
        StringAsStr,
        StringDrop,
        StringFmtWriteStr,
        StringFmtWriteChar,
        F64SpecToString,
        IntToString,
        BoolToString,
        CharToString,
        FormatArgumentNewDisplay,
        FormatArgsNew,
        FormatArgsToString,
        VecNew,
        VecWithCapacity,
        VecLen,
        VecCapacity,
        VecIsEmpty,
        VecPush,
        VecClear,
        VecAsSlice,
        VecAsMutSlice,
        VecIndex,
        VecIndexMut,
        VecRemove,
        VecExtend,
        VecDrop,
        BoxDrop,
        SliceSwap,
    ];
    let declined = [
        VecPop,
        VecReserve,
        VecSwap,
        StringFmtWriteFmt,
        StringFmtSpecWriteFmt,
    ];
    assert_eq!(lowered.len() + declined.len(), 39);
    for operation in lowered {
        let op = registry(operation)
            .unwrap_or_else(|| panic!("{operation:?} must have a registry entry"));
        assert_eq!(op.id, operation, "{operation:?} entry answers for itself");
    }
    for operation in declined {
        assert!(
            registry(operation).is_none(),
            "{operation:?} must be an exact decline in wave 1"
        );
    }
}

/// The proof gate accepts every supported signature, and `validate` proves emission at every
/// carrier the first slot admits.
#[test]
fn every_supported_operation_validates() {
    let (table, interns) = table();
    for (operation, _) in SUPPORTED {
        let signature = signature_of(*operation);
        let cx = lower_cx(*operation, &signature, &table, &interns);
        let op =
            validate(&cx).unwrap_or_else(|decline| panic!("{operation:?}: {}", decline.reason()));
        assert_eq!(op.id, *operation);
    }
}

/// A canonical `String` drop glue maps to `StringDrop`; a `Vec` glue is a wave-2 decline.
#[test]
fn drop_glue_resolves_through_the_candidate() {
    let (table, interns) = table();
    let by_ref = sig(vec![STRING_MUT], UNIT);
    let cx = LowerCx {
        semantic: BodySemantic::DropGlue(STRING),
        sig: &by_ref,
        table: &table,
        interns: &interns,
        element_glue: None,
    };
    let rewrite = rewrite(&cx, &[Carrier::StringBuilder]).expect("string drop glue lowers");
    assert_eq!(rewrite.operation, StandardOperation::StringDrop);

    let vec_sig = sig(vec![VEC_U8], UNIT);
    let cx = LowerCx {
        semantic: BodySemantic::DropGlue(VEC_U8),
        sig: &vec_sig,
        table: &table,
        interns: &interns,
        element_glue: None,
    };
    let vec_rewrite = super::rewrite(&cx, &[Carrier::NativeVec]).expect("vec drop glue lowers");
    assert_eq!(vec_rewrite.operation, StandardOperation::VecDrop);
}

/// A native Box keeps the canonical pointer shape. Its compiler glue either returns immediately
/// for a drop-free pointee or invokes the exact structural pointee glue before yielding the
/// allocation to JavaScript GC.
#[test]
fn box_drop_preserves_pointee_glue_without_allocator_machinery() {
    let (table, interns) = table();
    let plain_sig = sig(vec![BOX_U32_MUT], UNIT);
    let plain = LowerCx {
        semantic: BodySemantic::DropGlue(BOX_U32),
        sig: &plain_sig,
        table: &table,
        interns: &interns,
        element_glue: None,
    };
    let plain_rewrite = rewrite(&plain, &[Carrier::NativeBox]).expect("plain Box drop lowers");
    assert_eq!(plain_rewrite.operation, StandardOperation::BoxDrop);
    assert_eq!(
        jsast::stmt_to_string(&jsast::function(
            "f",
            plain_rewrite.parameters,
            plain_rewrite.statements
        ))
        .trim_end(),
        r#"function f($native0) {
  return;
}"#
    );

    let glue = JsName::new("pointee_drop".to_owned());
    let droppy_sig = sig(vec![BOX_DROPPY_MUT], UNIT);
    let droppy = LowerCx {
        semantic: BodySemantic::DropGlue(BOX_DROPPY),
        sig: &droppy_sig,
        table: &table,
        interns: &interns,
        element_glue: Some(&glue),
    };
    let rewrite = rewrite(&droppy, &[Carrier::NativeBox]).expect("structural Box drop lowers");
    assert_eq!(
        jsast::stmt_to_string(&jsast::function(
            "f",
            rewrite.parameters,
            rewrite.statements
        ))
        .trim_end(),
        r#"function f($native0) {
  const $nativeBox = $native0.buf[$native0.off];
  pointee_drop($nativeBox.buf[$nativeBox.off]);
  return;
}"#
    );
    assert!(rewrite.item_references.contains(&glue));
}

/// Missing structural glue is a transactional decline, and a custom pointee destructor is never
/// admitted by this carrier.
#[test]
fn box_drop_requires_proven_structural_pointee_glue() {
    let (table, interns) = table();
    let droppy_sig = sig(vec![BOX_DROPPY_MUT], UNIT);
    let droppy = LowerCx {
        semantic: BodySemantic::DropGlue(BOX_DROPPY),
        sig: &droppy_sig,
        table: &table,
        interns: &interns,
        element_glue: None,
    };
    assert_eq!(
        rewrite(&droppy, &[Carrier::NativeBox]).err(),
        Some(SemanticDecline::ElementDropRequired)
    );
    validate(&droppy).expect("validation uses a proof glue name");

    let own_sig = sig(vec![BOX_OWN_DROP_MUT], UNIT);
    let own = LowerCx {
        semantic: BodySemantic::DropGlue(BOX_OWN_DROP),
        sig: &own_sig,
        table: &table,
        interns: &interns,
        element_glue: None,
    };
    assert_eq!(
        rewrite(&own, &[Carrier::NativeBox]).err(),
        Some(SemanticDecline::ElementHasOwnDrop)
    );
}

/// A carrier a slot does not admit is refused, never mis-lowered.
#[test]
fn wrong_carrier_is_an_exact_decline() {
    let (table, interns) = table();
    let signature = signature_of(StandardOperation::StringPush);
    let cx = lower_cx(StandardOperation::StringPush, &signature, &table, &interns);
    assert_eq!(
        rewrite(&cx, &[Carrier::HostString]).err(),
        Some(SemanticDecline::WrongCarrier(SignatureSlot::Argument(0)))
    );
}

/// Every helper an emitter inserts is declared in its registry entry, so the cost model's
/// helper accounting can never under-count.
#[test]
fn emitted_helpers_are_declared() {
    let (table, interns) = table();
    for (operation, carriers) in SUPPORTED {
        let op = registry(*operation).unwrap();
        let signature = signature_of(*operation);
        let cx = lower_cx(*operation, &signature, &table, &interns);
        for carrier in *carriers {
            let selected: Vec<Carrier> = carrier
                .iter()
                .copied()
                .cycle()
                .take(op.slots.len())
                .collect();
            let rewrite = rewrite(&cx, &selected).unwrap();
            for helper in rewrite.runtime_helpers {
                assert!(
                    op.helpers.contains(&helper),
                    "{operation:?} emits __rt.{helper} but does not declare it"
                );
            }
        }
    }
}

/// A vec of structural drop-needing elements lowers `clear`/`drop` through the resolved glue,
/// element by element **in ascending order** with each slot cleared — and `clear` empties the
/// vector before any destructor runs.
#[test]
fn dropful_clear_and_drop_call_the_glue_in_order() {
    let (table, interns) = table();
    let glue = JsName::new("elem_drop".to_owned());
    let signature = sig(vec![VEC_DROPPY_MUT], UNIT);
    for (operation, expected) in [
        (
            StandardOperation::VecClear,
            r#"function f($native0) {
  const $nativeOldLen = $native0.len;
  $native0.len = 0;
  let $nativeIndex = 0;
  while ($nativeIndex < $nativeOldLen) {
    const $nativeElement = $native0.buf[$nativeIndex];
    elem_drop($nativeElement);
    $native0.buf[$nativeIndex] = undefined;
    $nativeIndex += 1;
  }
  return;
}"#,
        ),
        (
            StandardOperation::VecDrop,
            r#"function f($native0) {
  const $nativeOldLen = $native0.len;
  let $nativeIndex = 0;
  while ($nativeIndex < $nativeOldLen) {
    const $nativeElement = $native0.buf[$nativeIndex];
    elem_drop($nativeElement);
    $native0.buf[$nativeIndex] = undefined;
    $nativeIndex += 1;
  }
  $native0.len = 0;
  __rt.native_vec_set_capacity($native0, 0);
  return;
}"#,
        ),
    ] {
        let cx = LowerCx {
            semantic: BodySemantic::Standard(operation),
            sig: &signature,
            table: &table,
            interns: &interns,
            element_glue: Some(&glue),
        };
        let rewrite = rewrite(&cx, &[Carrier::NativeVec])
            .unwrap_or_else(|decline| panic!("{operation:?}: {}", decline.reason()));
        assert_eq!(
            jsast::stmt_to_string(&jsast::function(
                "f",
                rewrite.parameters,
                rewrite.statements
            ))
            .trim_end(),
            expected
        );
        assert!(rewrite.item_references.contains(&glue));
    }
}

/// A needy element without resolved glue is an exact `ElementDropRequired` decline in `rewrite`,
/// while `validate` still proves the emission with its stand-in glue (v1 `preflight`).
#[test]
fn element_glue_is_required_but_provable() {
    let (table, interns) = table();
    let signature = sig(vec![VEC_DROPPY_MUT], UNIT);
    let cx = lower_cx(StandardOperation::VecClear, &signature, &table, &interns);
    assert_eq!(
        rewrite(&cx, &[Carrier::NativeVec]).err(),
        Some(SemanticDecline::ElementDropRequired)
    );
    validate(&cx).expect("validation proves emission with the stand-in glue");
}

/// An element with its own destructor declines outright — with or without glue.
#[test]
fn own_drop_element_is_an_exact_decline() {
    let (table, interns) = table();
    let glue = JsName::new("elem_drop".to_owned());
    let signature = sig(vec![VEC_OWN_DROP_MUT], UNIT);
    for element_glue in [None, Some(&glue)] {
        let cx = LowerCx {
            semantic: BodySemantic::Standard(StandardOperation::VecClear),
            sig: &signature,
            table: &table,
            interns: &interns,
            element_glue,
        };
        assert_eq!(
            rewrite(&cx, &[Carrier::NativeVec]).err(),
            Some(SemanticDecline::ElementHasOwnDrop)
        );
    }
}

/// The bounds operations REQUIRE the track-caller ABI, everything else refuses it (v1
/// `validate_abi`).
#[test]
fn caller_location_is_per_operation() {
    let (table, interns) = table();
    // `VecIndex` without the Location parameter refuses.
    let plain = sig(vec![VEC_U32_REF, USIZE], U32_REF);
    let cx = lower_cx(StandardOperation::VecIndex, &plain, &table, &interns);
    assert_eq!(
        validate(&cx).err(),
        Some(SemanticDecline::CallerLocationAbi)
    );
    // `VecLen` with one refuses too.
    let tracked = sig_tracked(vec![VEC_U32_REF], USIZE);
    let cx = lower_cx(StandardOperation::VecLen, &tracked, &table, &interns);
    assert_eq!(
        validate(&cx).err(),
        Some(SemanticDecline::CallerLocationAbi)
    );
}

/// A reference to an indirect aggregate is the aggregate itself, not a slot record around it.
#[test]
fn vec_index_returns_the_element_reference_form() {
    let (table, interns) = table();
    let signature = sig_tracked(vec![VEC_DROPPY_REF, USIZE], DROPPY_REF);
    let cx = lower_cx(StandardOperation::VecIndex, &signature, &table, &interns);
    let rewrite = rewrite(&cx, &[Carrier::NativeVec]).expect("aggregate Vec index lowers");
    assert_eq!(
        jsast::stmt_to_string(&jsast::function(
            "f".to_owned(),
            rewrite.parameters,
            rewrite.statements,
        ))
        .trim_end(),
        r#"function f($native0, $native1, $native2) {
  if ($native1 >= $native0.len) {
    throw new RangeError("rustc_codegen_js: native Vec index out of bounds");
  }
  return $native0.buf[$native1];
}"#
    );
}

/// A zero-sized element has no native capacity story: `with_capacity` declines exactly.
#[test]
fn zero_sized_element_declines() {
    let (table, interns) = table();
    let signature = sig(vec![USIZE], VEC_ZST);
    let cx = lower_cx(
        StandardOperation::VecWithCapacity,
        &signature,
        &table,
        &interns,
    );
    assert_eq!(validate(&cx).err(), Some(SemanticDecline::ZeroSizedElement));
}

mod structural {
    //! The structural drop chain: needy fields in declaration order, plain shapes only.

    use super::*;
    use crate::lower::semantic_ops::drop::{structural_chain, structural_drop_body, ChainField};

    #[test]
    fn chain_lists_needy_fields_in_declaration_order() {
        let (table, interns) = table();
        assert_eq!(
            structural_chain(&table, &interns, HOLDER).expect("a plain struct chains"),
            vec![
                ChainField {
                    member: "first".to_owned(),
                    ty: VEC_DROPPY,
                },
                ChainField {
                    member: "second".to_owned(),
                    ty: DROPPY,
                },
            ]
        );
    }

    #[test]
    fn own_destructor_and_candidate_types_do_not_chain() {
        let (table, interns) = table();
        // Its own destructor: the shim is not structural.
        assert!(structural_chain(&table, &interns, OWN_DROP).is_err());
        // A native candidate drops through its registry op, not a chain.
        assert!(structural_chain(&table, &interns, VEC_DROPPY).is_err());
    }

    #[test]
    fn body_calls_each_glue_in_chain_order() {
        let chain = vec![
            ("first".to_owned(), JsName::new("drop_first".to_owned())),
            ("second".to_owned(), JsName::new("drop_second".to_owned())),
        ];
        let body = structural_drop_body(&chain);
        let rendered =
            jsast::stmt_to_string(&jsast::function("f", vec!["$native0".to_owned()], body));
        assert_eq!(
            rendered.trim_end(),
            r#"function f($native0) {
  drop_first($native0.first);
  drop_second($native0.second);
  return;
}"#
        );
    }
}

mod adapters {
    //! The string `Convert` adapters (D6) over the same canonical `String` tree: member paths
    //! resolved from the type table, one evaluation of the converted value, fresh blocks on the
    //! way into baseline.

    use super::*;
    use crate::lower::js::adapters::convert;

    fn converted(value: jsast::Expr, from: Carrier, to: Carrier) -> String {
        let (table, interns) = table();
        jsast::expr_to_string(
            &convert(&table, &interns, value, STRING, from, to)
                .unwrap_or_else(|decline| panic!("{from:?} -> {to:?}: {decline:?}")),
        )
    }

    #[test]
    fn builder_to_host_reads_text() {
        assert_eq!(
            converted(jsast::id("s"), Carrier::StringBuilder, Carrier::HostString),
            "s.text"
        );
    }

    #[test]
    fn host_to_builder_measures_utf8_capacity() {
        assert_eq!(
            converted(jsast::id("s"), Carrier::HostString, Carrier::StringBuilder),
            "__rt.convert_host_sb(s)"
        );
    }

    #[test]
    fn baseline_to_host_decodes_the_block() {
        assert_eq!(
            converted(jsast::id("s"), Carrier::Baseline, Carrier::HostString),
            "__rt.bytes_str(s.vec.buf.inner.ptr.buf, s.vec.buf.inner.ptr.off, s.vec.len)"
        );
    }

    #[test]
    fn baseline_to_host_evaluates_a_compound_value_once() {
        let value = jsast::call(jsast::id("g"), vec![]);
        assert_eq!(
            converted(value, Carrier::Baseline, Carrier::HostString),
            "(($cv) => __rt.bytes_str($cv.vec.buf.inner.ptr.buf, $cv.vec.buf.inner.ptr.off, \
             $cv.vec.len))(g())"
        );
    }

    #[test]
    fn host_to_baseline_builds_a_fresh_block() {
        assert_eq!(
            converted(jsast::id("s"), Carrier::HostString, Carrier::Baseline),
            "(($cb) => ({ vec: { buf: { inner: { ptr: { buf: $cb.buf, off: 0 }, cap: $cb.cap, \
             alloc: undefined }, _marker: undefined }, len: $cb.len } }))\
             (__rt.convert_str_block(s))"
        );
    }

    #[test]
    fn builder_to_baseline_carries_the_capacity() {
        assert_eq!(
            converted(jsast::id("s"), Carrier::StringBuilder, Carrier::Baseline),
            "(($cb) => ({ vec: { buf: { inner: { ptr: { buf: $cb.buf, off: 0 }, cap: $cb.cap, \
             alloc: undefined }, _marker: undefined }, len: $cb.len } }))\
             (__rt.convert_sb_block(s))"
        );
    }

    #[test]
    fn baseline_to_builder_carries_the_cap_leaf() {
        assert_eq!(
            converted(jsast::id("s"), Carrier::Baseline, Carrier::StringBuilder),
            "{ text: __rt.bytes_str(s.vec.buf.inner.ptr.buf, s.vec.buf.inner.ptr.off, \
             s.vec.len), cap: s.vec.buf.inner.cap }"
        );
    }

    #[test]
    fn native_vec_to_baseline_registers_a_capacity_true_block() {
        // The pointer is a REGISTERED heap block (`unscale` answers from the block registry)
        // and the Rust-visible capacity is the carrier's, not the length (D5).
        assert_eq!(
            converted_at(
                jsast::id("v"),
                VEC_U32,
                Carrier::NativeVec,
                Carrier::Baseline
            ),
            "{ buf: { inner: { ptr: __rt.convert_nv_block(v, 4), cap: v.cap, \
             alloc: undefined }, _marker: undefined }, len: v.len }"
        );
    }

    #[test]
    fn baseline_to_native_vec_preserves_the_capacity() {
        assert_eq!(
            converted_at(
                jsast::id("v"),
                VEC_U32,
                Carrier::Baseline,
                Carrier::NativeVec
            ),
            "__rt.convert_block_nv(v.buf.inner.ptr.buf, v.buf.inner.ptr.off, v.len, \
             v.buf.inner.cap)"
        );
    }

    #[test]
    fn baseline_vec_to_slice_view_borrows_the_initialized_range() {
        assert_eq!(
            converted_at(
                jsast::id("v"),
                VEC_U32,
                Carrier::Baseline,
                Carrier::SliceView
            ),
            "{ buf: v.buf.inner.ptr.buf, off: v.buf.inner.ptr.off, len: v.len }"
        );
    }

    #[test]
    fn native_vec_to_slice_view_shares_the_buffer() {
        assert_eq!(
            converted_at(
                jsast::id("v"),
                VEC_U32,
                Carrier::NativeVec,
                Carrier::SliceView
            ),
            "{ buf: v.buf, off: 0, len: v.len }"
        );
    }

    #[test]
    fn slice_view_to_native_vec_copies_the_window() {
        assert_eq!(
            converted_at(
                jsast::id("s"),
                SLICE_REF,
                Carrier::SliceView,
                Carrier::NativeVec
            ),
            "__rt.convert_sv_nv(s)"
        );
    }

    #[test]
    fn slice_view_to_baseline_re_records() {
        assert_eq!(
            converted_at(
                jsast::id("s"),
                SLICE_REF,
                Carrier::SliceView,
                Carrier::Baseline
            ),
            "{ buf: s.buf, off: s.off, len: s.len }"
        );
    }

    #[test]
    fn native_vec_to_baseline_evaluates_a_compound_value_once() {
        let value = jsast::call(jsast::id("g"), vec![]);
        assert_eq!(
            converted_at(value, VEC_U32, Carrier::NativeVec, Carrier::Baseline),
            "(($cv) => ({ buf: { inner: { ptr: __rt.convert_nv_block($cv, 4), \
             cap: $cv.cap, alloc: undefined }, _marker: undefined }, len: $cv.len }))(g())"
        );
    }

    #[test]
    fn baseline_to_slice_view_stays_a_decline() {
        let (table, interns) = table();
        assert!(convert(
            &table,
            &interns,
            jsast::id("v"),
            SLICE_REF,
            Carrier::Baseline,
            Carrier::SliceView
        )
        .is_err());
    }

    fn converted_at(value: jsast::Expr, ty: TypeId, from: Carrier, to: Carrier) -> String {
        let (table, interns) = table();
        jsast::expr_to_string(
            &convert(&table, &interns, value, ty, from, to)
                .unwrap_or_else(|decline| panic!("{from:?} -> {to:?}: {decline:?}")),
        )
    }
}

#[test]
fn host_string_carriers_compose_with_loop_fields_and_edge_materialization() {
    assert_carrier_composition(false, None);
}

#[test]
fn slice_view_carriers_compose_with_loop_fields_and_shared_storage() {
    assert_carrier_composition(true, None);
}

#[test]
fn string_builder_carriers_compose_with_owned_copies_mutation_and_drop() {
    assert_carrier_composition(true, Some(Carrier::StringBuilder));
}

#[test]
fn native_vec_carriers_compose_with_owned_copies_mutation_and_drop() {
    assert_carrier_composition(true, Some(Carrier::NativeVec));
}

fn assert_carrier_composition(with_slice_view: bool, owned_carrier: Option<Carrier>) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use crate::jir::origin::OriginId;
    use crate::jir::repr::{FuncBuilder, OpKind, RTerm, Root};
    use crate::jir::semantic::{BinaryOp, Body, BodyKind, CallSemantic, Callee, ConstTree, ItemId};
    use crate::jir::summary::{Availability, ItemClass, ItemSummary, SemanticKey};
    use crate::lower::{lower_func, LowerInput};
    use crate::opt::{self, OptStats};
    use crate::opts::{OptimizerMode, Opts};
    use rcgjs_object::object::{BlobRef, ItemKind, Linkage};

    const PAIR: TypeId = TypeId(56);
    const ORIGIN: OriginId = OriginId(0);
    let (mut table, mut interns) = table();
    let fields = vec![field(&mut interns, "x", I32), field(&mut interns, "y", I32)];
    let mut pair = strukt(&mut interns, PAIR, "Pair", fields, 8);
    pair.js.needs_clone = true;
    table.insert(pair);
    let observe = interns.intern_str("observe");
    let mutate = interns.intern_str("mutate_owned");
    let release = interns.intern_str("release_owned");
    let owned_type = owned_carrier.map(|carrier| match carrier {
        Carrier::StringBuilder => STRING,
        Carrier::NativeVec => VEC_U32,
        _ => panic!("unsupported owned carrier {carrier:?}"),
    });
    let owned_reference = owned_type.map(|ty| {
        if ty == STRING {
            STRING_MUT
        } else {
            VEC_U32_MUT
        }
    });
    let item = ItemId {
        definition: def_id(100),
        instantiation: 0,
    };
    let mut parameters = vec![STRING, PAIR, I32];
    if with_slice_view {
        parameters.push(SLICE_REF);
    }
    parameters.extend(owned_type);
    let mut b = FuncBuilder::new(item, sig(parameters, STRING));
    let entry = FuncBuilder::ENTRY;
    let text = b.param(STRING, ORIGIN);
    let caller_pair = b.param(PAIR, ORIGIN);
    let rounds = b.param(I32, ORIGIN);
    let view = with_slice_view.then(|| b.param(SLICE_REF, ORIGIN));
    let owned = owned_type.map(|ty| b.param(ty, ORIGIN));
    let copied_view = view.map(|view| b.op(entry, OpKind::Copy(view), SLICE_REF, ORIGIN));
    let moved_view = copied_view.map(|view| b.op(entry, OpKind::Move(view), SLICE_REF, ORIGIN));
    let copied_owned = owned
        .zip(owned_type)
        .map(|(value, ty)| b.op(entry, OpKind::Copy(value), ty, ORIGIN));
    let moved_owned = copied_owned
        .zip(owned_type)
        .map(|(value, ty)| b.op(entry, OpKind::Move(value), ty, ORIGIN));
    let baseline = b.op(
        entry,
        OpKind::Convert {
            v: text,
            from: Carrier::HostString,
            to: Carrier::Baseline,
        },
        STRING,
        ORIGIN,
    );
    let restored = b.op(
        entry,
        OpKind::Convert {
            v: baseline,
            from: Carrier::Baseline,
            to: Carrier::HostString,
        },
        STRING,
        ORIGIN,
    );
    let zero = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let one = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        ORIGIN,
    );
    let local = b.op(
        entry,
        OpKind::Aggregate {
            ty: PAIR,
            variant: 0,
            active_field: None,
            fields: vec![zero, one],
        },
        PAIR,
        ORIGIN,
    );
    let saved = b.op(
        entry,
        OpKind::Field {
            base: local,
            variant: None,
            field: 0,
        },
        I32,
        ORIGIN,
    );
    let header = b.block();
    let latch = b.block();
    let choose = b.block();
    let left = b.block();
    let right = b.block();
    let merge = b.block();
    b.terminate(entry, RTerm::Goto(header, vec![]));
    let current = b.op(
        header,
        OpKind::Field {
            base: local,
            variant: None,
            field: 0,
        },
        I32,
        ORIGIN,
    );
    let more = b.op(
        header,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: current,
            r: rounds,
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
            cases: vec![(0, choose, vec![])],
            default: (latch, vec![]),
        },
    );
    let next = b.op(
        latch,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: current,
            r: one,
            ty: I32,
        },
        I32,
        ORIGIN,
    );
    b.effect(
        latch,
        OpKind::SetField {
            base: local,
            variant: None,
            field: 0,
            v: next,
        },
        ORIGIN,
    );
    b.terminate(latch, RTerm::Goto(header, vec![]));
    b.terminate(
        choose,
        RTerm::Switch {
            scrut: rounds,
            scrut_ty: I32,
            cases: vec![(0, left, vec![])],
            default: (right, vec![]),
        },
    );
    let mut left_args = vec![caller_pair];
    let mut right_args = vec![local];
    left_args.extend(view);
    right_args.extend(moved_view);
    left_args.extend(owned);
    right_args.extend(moved_owned);
    b.terminate(left, RTerm::Goto(merge, left_args));
    b.terminate(right, RTerm::Goto(merge, right_args));
    let merged = b.block_param(merge, PAIR, ORIGIN);
    let merged_view = view.map(|_| b.block_param(merge, SLICE_REF, ORIGIN));
    let merged_owned = owned_type.map(|ty| b.block_param(merge, ty, ORIGIN));
    let mut observed = vec![merged, saved];
    if let Some(view) = merged_view {
        observed.push(b.op(
            merge,
            OpKind::Convert {
                v: view,
                from: Carrier::SliceView,
                to: Carrier::Baseline,
            },
            SLICE_REF,
            ORIGIN,
        ));
    }
    let borrowed_owned = merged_owned.zip(owned_reference).map(|(value, ty)| {
        b.op(
            merge,
            OpKind::BorrowView {
                root: Root::Value(value),
                path: vec![],
                mutable: true,
            },
            ty,
            ORIGIN,
        )
    });
    if let Some((borrow, reference)) = borrowed_owned.zip(owned_reference) {
        let alias = b.op(merge, OpKind::Copy(borrow), reference, ORIGIN);
        let (bits, ty) = if owned_carrier == Some(Carrier::StringBuilder) {
            (0x1f600, CHAR)
        } else {
            (77, U32)
        };
        let value = b.op(
            merge,
            OpKind::Const(ConstTree::Scalar { bits, bytes: 4 }),
            ty,
            ORIGIN,
        );
        for receiver in [alias, borrow] {
            b.op(
                merge,
                OpKind::Call {
                    callee: Callee::Direct {
                        item: ItemId {
                            definition: def_id(102),
                            instantiation: 0,
                        },
                        link_name: mutate,
                    },
                    function: None,
                    args: vec![receiver, value],
                    track_caller: None,
                    semantic: CallSemantic::Ordinary,
                    host: None,
                },
                UNIT,
                ORIGIN,
            );
        }
        observed.push(b.op(
            merge,
            OpKind::Convert {
                v: merged_owned.unwrap(),
                from: owned_carrier.unwrap(),
                to: Carrier::Baseline,
            },
            owned_type.unwrap(),
            ORIGIN,
        ));
    }
    b.op(
        merge,
        OpKind::Call {
            callee: Callee::Direct {
                item: ItemId {
                    definition: def_id(101),
                    instantiation: 0,
                },
                link_name: observe,
            },
            function: None,
            args: observed,
            track_caller: None,
            semantic: CallSemantic::Ordinary,
            host: None,
        },
        I32,
        ORIGIN,
    );
    if let Some(borrow) = borrowed_owned {
        b.op(
            merge,
            OpKind::Call {
                callee: Callee::Direct {
                    item: ItemId {
                        definition: def_id(103),
                        instantiation: 0,
                    },
                    link_name: release,
                },
                function: None,
                args: vec![borrow],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            UNIT,
            ORIGIN,
        );
    }
    b.terminate(merge, RTerm::Return(Some(restored)));
    let mut original = b.finish();
    for value in [text, restored] {
        original.values[value.0 as usize].carrier = Carrier::HostString;
    }
    for value in [view, copied_view, moved_view, merged_view]
        .into_iter()
        .flatten()
    {
        original.values[value.0 as usize].carrier = Carrier::SliceView;
    }
    for value in [owned, copied_owned, moved_owned, merged_owned]
        .into_iter()
        .flatten()
    {
        original.values[value.0 as usize].carrier = owned_carrier.unwrap();
    }
    let policy = opt::observed_carriers(&original);
    original.verify_carriers(&table, policy).unwrap();
    let mut optimized = original.clone();
    let opts = Opts {
        optimizer: OptimizerMode::On,
        ..Opts::default()
    };
    let mut stats = OptStats::default();
    opt::optimize(&mut optimized, &table, &interns, &opts, &mut stats).unwrap();
    assert!(
        stats.pea.scalarized > 0,
        "mixed carrier function must run PEA (SliceView={with_slice_view}, owned={owned_carrier:?}): {stats:?}"
    );
    assert!(
        stats.pea.params_added > 0,
        "loop fields must cross generated parameters"
    );
    optimized.verify_carriers(&table, policy).unwrap();
    assert!(
        !optimized.blocks[0]
            .ops
            .iter()
            .any(|op| { matches!(op.kind, OpKind::Aggregate { ty: PAIR, .. }) }),
        "the local pair must remain virtual before the loop"
    );
    assert!(
        optimized
            .blocks
            .iter()
            .skip(1)
            .flat_map(|block| &block.ops)
            .any(|op| { matches!(op.kind, OpKind::Aggregate { ty: PAIR, .. }) }),
        "the escaping edge must reconstruct the current fields"
    );
    assert_eq!(
        optimized
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter(|op| { matches!(op.kind, OpKind::Convert { .. }) })
            .count(),
        2 + usize::from(with_slice_view) + usize::from(owned_carrier.is_some()),
        "the real String and slice adapters survive"
    );

    let mut report = original.clone();
    let mut report_stats = OptStats::default();
    let report_opts = Opts {
        optimizer: OptimizerMode::Report,
        ..Opts::default()
    };
    opt::optimize(
        &mut report,
        &table,
        &interns,
        &report_opts,
        &mut report_stats,
    )
    .unwrap();
    assert_eq!(report, original, "report remains non-emitting");
    assert_eq!(
        report_stats, stats,
        "report and on analyze the same composition"
    );

    let body = Body {
        item,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: original.sig.clone(),
        locals: vec![],
        blocks: vec![],
    };
    let summary = ItemSummary {
        key: SemanticKey {
            link_name: "f".to_owned(),
            item: Some(item),
        },
        body_hash: None,
        semantic_hash: None,
        availability: Availability::Available,
        deps: vec!["observe".to_owned()],
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
    let mut program = include_str!("../../../../runtime/shim.js").to_owned();
    if let Some(carrier) = owned_carrier {
        let operations = if carrier == Carrier::StringBuilder {
            [StandardOperation::StringPush, StandardOperation::StringDrop]
        } else {
            [StandardOperation::VecPush, StandardOperation::VecDrop]
        };
        for (name, operation) in ["mutate_owned", "release_owned"]
            .into_iter()
            .zip(operations)
        {
            let signature = signature_of(operation);
            let cx = lower_cx(operation, &signature, &table, &interns);
            let rewrite = rewrite(&cx, &[carrier]).unwrap();
            program.push_str(&jsast::stmt_to_string(&jsast::function(
                name,
                rewrite.parameters,
                rewrite.statements,
            )));
        }
    }
    for (name, func) in [("before", &original), ("after", &optimized)] {
        let name = JsName::new(name.to_owned());
        let input = LowerInput {
            name: &name,
            summary: &summary,
            body: &body,
            origins: None,
            interns: &interns,
            table: &table,
            opts: &opts,
            items: crate::lower::js::host::EMPTY_ITEMS,
            drop_glue: crate::lower::js::host::EMPTY_DROP_GLUE,
            vtables: &crate::lower::js::unsize::EMPTY_VTABLES,
            spills: &[],
            abi: None,
            physical: None,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
        };
        program.push_str(&jsast::stmt_to_string(
            &lower_func(&input, func).unwrap().decl,
        ));
    }
    program.push_str(&format!("const expectView = {with_slice_view};\n"));
    program.push_str(&format!(
        "const ownedKind = {};\n",
        match owned_carrier {
            Some(Carrier::StringBuilder) => "'builder'",
            Some(Carrier::NativeVec) => "'vec'",
            _ => "null",
        }
    ));
    program.push_str(
        r#"
const assert = require('node:assert/strict');
let seen;
let throwAtObserve = false;
const thrown = { observation: 'failed' };
function observe(pair, saved, view, owned) {
  seen = { pair, fields: [pair.x, pair.y], saved, view, owned };
  pair.x = -100;
  if (throwAtObserve) throw thrown;
  return 0;
}
for (const f of [before, after]) {
  for (const text of ['', 'plain', '\u00e9\u6f22\ud83d\ude00', 'a\0b', '\ufefftext', '\ufeff']) {
    for (const rounds of [0, 1, 2, 17]) {
     for (const reserve of [0, 16]) {
      for (const shouldThrow of [false, true]) {
      const caller = { x: 99, y: 42 };
      const buffer = [10, 20, 30, 40];
      const view = { buf: buffer, off: 1, len: 2 };
      const capacity = (ownedKind === 'builder' ? __rt.str_len(text) : 2) + reserve;
      const owned = ownedKind === 'builder' ? { text, cap: capacity } : ownedKind === 'vec' ? { buf: [1, 2], len: 2, cap: capacity } : undefined;
      if (ownedKind === 'vec') owned.buf.length = capacity;
      throwAtObserve = shouldThrow;
      if (shouldThrow) assert.throws(() => f(text, caller, rounds, view, owned), error => error === thrown);
      else assert.equal(f(text, caller, rounds, view, owned), text);
      assert.deepEqual(seen.fields, rounds === 0 ? [99, 42] : [rounds, 1]);
      assert.equal(seen.saved, 0);
      assert.equal(seen.pair === caller, rounds === 0);
      assert.equal(caller.x, rounds === 0 ? -100 : 99);
      assert.equal(seen.view !== undefined, expectView);
      if (expectView) {
        assert.notEqual(seen.view, view);
        assert.equal(seen.view.buf, buffer);
        assert.equal(seen.view.off, 1);
        assert.equal(seen.view.len, 2);
        seen.view.off = 0;
        seen.view.len = 4;
        assert.equal(view.off, 1);
        assert.equal(view.len, 2);
        buffer[1] = 88;
        assert.equal(seen.view.buf[1], 88);
      }
      assert.equal(seen.owned !== undefined, ownedKind !== null);
      if (ownedKind !== null) {
        const vec = ownedKind === 'builder' ? seen.owned.vec : seen.owned;
        const ptr = vec.buf.inner.ptr;
        const dropped = rounds === 0 && !shouldThrow;
        const mutated = rounds === 0 && shouldThrow;
        const expectedCapacity = Math.max(capacity, ownedKind === 'builder' ? __rt.str_len(text) + 8 : 4);
        if (ownedKind === 'builder') {
          assert.equal(__rt.bytes_str(ptr.buf, ptr.off, vec.len), text + '\ud83d\ude00\ud83d\ude00');
          assert.equal(owned.text, dropped ? '' : mutated ? text + '\ud83d\ude00\ud83d\ude00' : text);
        } else {
          assert.deepEqual(ptr.buf.slice(ptr.off, ptr.off + vec.len), [1, 2, 77, 77]);
          assert.equal(owned.len, dropped ? 0 : mutated ? 4 : 2);
          assert.equal(owned.buf.length, dropped ? 0 : mutated ? expectedCapacity : capacity);
          if (rounds !== 0) assert.deepEqual(owned.buf.slice(0, 2), [1, 2]);
        }
        assert.equal(vec.buf.inner.cap, expectedCapacity);
        assert.equal(owned.cap, dropped ? 0 : mutated ? expectedCapacity : capacity);
      }
      }
     }
    }
  }
}
"#,
    );
    let mut child = Command::new("node")
        .arg("--input-type=commonjs")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node is required for emitted carrier-composition verification");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
