//! Unit tests: every supported operation lowered at every legal carrier against the pinned v1
//! spelling, the registry table, and the string and vec `Convert` adapters over a
//! hand-built canonical `String`/`Vec` type tree.

#![allow(
    clippy::too_many_lines,
    reason = "unit tests construct full semantic-op type tables inline"
)]

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
const OPTION_U32: TypeId = TypeId(90);
const RANGE_USIZE: TypeId = TypeId(91);
const ARR_U32_3: TypeId = TypeId(92);
const BOX_ARR_U32_3: TypeId = TypeId(93);
const RAW_U32_CONST: TypeId = TypeId(94);
const RAW_U32_MUT: TypeId = TypeId(95);
const SLICE_ITER_U32: TypeId = TypeId(96);
const SLICE_ITER_MUT_U32: TypeId = TypeId(97);

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

    let none_name = interns.intern_str("None");
    let some_name = interns.intern_str("Some");
    let mut opt_u32 = descriptor(
        OPTION_U32,
        TypeKind::Adt {
            def: crate::jir::semantic::DefinitionId {
                crate_id: 1,
                local_hash: OPTION_U32.0 as u64,
            },
            args: vec![U32],
            form: NominalForm::Enum,
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
                    fields: vec![field(&mut interns, "_0", U32)],
                    logical_discr: 1,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
            ],
            niche: None,
        },
        8,
    );
    opt_u32.js.enum_repr = Some(crate::jir::types::EnumRepr::Tag { fieldless: false });
    put(opt_u32, &mut table);

    let range_fields = vec![
        field(&mut interns, "start", USIZE),
        field(&mut interns, "end", USIZE),
    ];
    put(
        strukt(&mut interns, RANGE_USIZE, "Range", range_fields, 8),
        &mut table,
    );

    put(
        descriptor(
            ARR_U32_3,
            TypeKind::Array {
                element: U32,
                length: 3,
            },
            12,
        ),
        &mut table,
    );
    put(box_of(&mut interns, BOX_ARR_U32_3, ARR_U32_3), &mut table);
    put(
        descriptor(
            RAW_U32_CONST,
            TypeKind::RawPointer {
                pointee: U32,
                mutable: false,
            },
            4,
        ),
        &mut table,
    );
    put(
        descriptor(
            RAW_U32_MUT,
            TypeKind::RawPointer {
                pointee: U32,
                mutable: true,
            },
            4,
        ),
        &mut table,
    );
    let iter_fields = vec![
        field(&mut interns, "ptr", RAW_U32_CONST),
        field(&mut interns, "end_or_len", RAW_U32_CONST),
        field(&mut interns, "_marker", MARKER_ZST),
    ];
    let mut iter_desc = strukt(&mut interns, SLICE_ITER_U32, "Iter", iter_fields, 8);
    iter_desc.meta.iterator = crate::jir::types::IteratorIdentity::Slice;
    put(iter_desc, &mut table);
    let iter_mut_fields = vec![
        field(&mut interns, "ptr", RAW_U32_CONST),
        field(&mut interns, "end_or_len", RAW_U32_MUT),
        field(&mut interns, "_marker", MARKER_ZST),
    ];
    let mut iter_mut_desc = strukt(
        &mut interns,
        SLICE_ITER_MUT_U32,
        "IterMut",
        iter_mut_fields,
        8,
    );
    iter_mut_desc.meta.iterator = crate::jir::types::IteratorIdentity::SliceMut;
    put(iter_mut_desc, &mut table);

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
            inline_never: false,
            inline_hint: false,
            inline_always: false,
        },
    }
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
        native_string_enabled: false,
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
        native_string_enabled: false,
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
  const $nativeBox = $native0.$b[$native0.$o];
  pointee_drop($nativeBox.$b[$nativeBox.$o]);
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
        native_string_enabled: false,
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
        native_string_enabled: false,
    };
    assert_eq!(
        rewrite(&own, &[Carrier::NativeBox]).err(),
        Some(SemanticDecline::ElementHasOwnDrop)
    );
}

/// Runs `harness` in Node after the shim and `functions`, failing the test on any throw.
fn run_in_node(functions: &[String], harness: &str) {
    let mut program = include_str!("../../../../runtime/shim.js").to_owned();
    for function in functions {
        program.push_str(function);
        program.push('\n');
    }
    program.push_str(harness);
    let output = test_node::run(test_node::Node::StdinCommonJs(&program))
        .expect("Node is required for emitted format-kernel verification");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        program
            .lines()
            .rev()
            .take(200)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn universal_native_string_fmt_kernel_lowers_all_kinds_and_int_tag_result() {
    use crate::analysis::heap::HeapPlan;
    use crate::item::{LinkItem, Linkage as ItemLinkage};
    use crate::jir::origin::OriginId;
    use crate::jir::repr::{FuncBuilder, OpKind, Proj, RTerm, Root, ValueId};
    use crate::jir::semantic::{Body, BodyKind, CallSemantic, Callee, ConstTree, ItemId};
    use crate::jir::summary::{Availability, ItemClass, ItemSummary, SemanticKey};
    use crate::jir::types::EnumRepr;
    use crate::lower::js::host::{ItemEntry, ItemIndex};
    use crate::lower::{lower_func, LowerInput};
    use crate::opts::Opts;
    use rcgjs_object::object::{BlobRef, ItemKind, Linkage};
    use std::collections::BTreeMap;

    const ORIGIN: OriginId = OriginId(0);
    const ARG_LIST_FMT: TypeId = TypeId(70);
    const ARG_LIST_FMT_REF: TypeId = TypeId(71);
    const TEMPLATE_FMT: TypeId = TypeId(72);
    const ARG_LIST_WR: TypeId = TypeId(73);
    const ARG_LIST_WR_REF: TypeId = TypeId(74);
    const TEMPLATE_WR: TypeId = TypeId(75);
    const WRAPPER: TypeId = TypeId(76);
    const WRAPPER_REF: TypeId = TypeId(77);
    const FMT_RESULT_INT: TypeId = TypeId(78);

    // Template 1 (`format!`):
    // "café={:+05} hex={:08x} HEX={:#08X} f={:.5} s={} ds={} str={} ch={} b={} 🦀"
    let mut tpl_fmt: Vec<u8> = Vec::new();
    let push_lit = |buf: &mut Vec<u8>, s: &str| {
        let bytes = s.as_bytes();
        assert!(bytes.len() <= 0x7f);
        buf.push(bytes.len() as u8);
        buf.extend_from_slice(bytes);
    };
    let push_spec = |buf: &mut Vec<u8>, flags: u32, width: Option<u16>, prec: Option<u16>| {
        let mut hdr = 0xc0u8 | 1;
        if width.is_some() {
            hdr |= 2;
        }
        if prec.is_some() {
            hdr |= 4;
        }
        buf.push(hdr);
        buf.extend_from_slice(&flags.to_le_bytes());
        if let Some(w) = width {
            buf.extend_from_slice(&w.to_le_bytes());
        }
        if let Some(p) = prec {
            buf.extend_from_slice(&p.to_le_bytes());
        }
    };
    const DEFAULT_FLAGS: u32 = (' ' as u32) | (3 << 29);
    const SIGN_PLUS: u32 = 1 << 21;
    const ALTERNATE: u32 = 1 << 23;
    const ZERO_PAD: u32 = 1 << 24;
    const WIDTH: u32 = 1 << 27;
    const PRECISION: u32 = 1 << 28;

    push_lit(&mut tpl_fmt, "café=");
    push_spec(
        &mut tpl_fmt,
        DEFAULT_FLAGS | SIGN_PLUS | ZERO_PAD | WIDTH,
        Some(5),
        None,
    );
    push_lit(&mut tpl_fmt, " hex=");
    push_spec(
        &mut tpl_fmt,
        DEFAULT_FLAGS | ZERO_PAD | WIDTH,
        Some(8),
        None,
    );
    push_lit(&mut tpl_fmt, " HEX=");
    push_spec(
        &mut tpl_fmt,
        DEFAULT_FLAGS | ALTERNATE | ZERO_PAD | WIDTH,
        Some(8),
        None,
    );
    push_lit(&mut tpl_fmt, " f=");
    push_spec(&mut tpl_fmt, DEFAULT_FLAGS | PRECISION, None, Some(5));
    push_lit(&mut tpl_fmt, " s=");
    tpl_fmt.push(0xc0);
    push_lit(&mut tpl_fmt, " ds=");
    tpl_fmt.push(0xc0);
    push_lit(&mut tpl_fmt, " str=");
    tpl_fmt.push(0xc0);
    push_lit(&mut tpl_fmt, " ch=");
    tpl_fmt.push(0xc0);
    push_lit(&mut tpl_fmt, " b=");
    tpl_fmt.push(0xc0);
    push_lit(&mut tpl_fmt, " 🦀");
    tpl_fmt.push(0);

    // Template 2 (`write!` with IntTag Result): "λ={:.5}:{}\n"
    let mut tpl_wr: Vec<u8> = Vec::new();
    push_lit(&mut tpl_wr, "λ=");
    push_spec(&mut tpl_wr, DEFAULT_FLAGS | PRECISION, None, Some(5));
    push_lit(&mut tpl_wr, ":");
    tpl_wr.push(0xc0);
    push_lit(&mut tpl_wr, "\n");
    tpl_wr.push(0);

    let (mut type_table, mut interns) = table();
    // Struct holding a `String` field to exercise `Proj::Field` on `DisplayKind::String`.
    let wrap_fields = vec![field(&mut interns, "label", STRING)];
    type_table.insert(strukt(&mut interns, WRAPPER, "Wrap", wrap_fields, 12));
    type_table.insert(reference(WRAPPER_REF, WRAPPER, false));
    // `Result<(), fmt::Error>` specialized to Phase 4 `EnumRepr::IntTag { fieldless: true }`.
    let mut int_tag_res = type_table.get(FMT_RESULT).unwrap().clone();
    int_tag_res.id = FMT_RESULT_INT;
    int_tag_res.js.peeled = FMT_RESULT_INT;
    int_tag_res.js.is_indirect = false;
    int_tag_res.js.enum_repr = Some(EnumRepr::IntTag { fieldless: true });
    type_table.insert(int_tag_res);

    let mut mk_arr = |id: TypeId, ref_id: TypeId, elem: TypeId, len: usize| {
        let mut arr = descriptor(
            id,
            TypeKind::Array {
                element: elem,
                length: len as u64,
            },
            (8 * len) as u64,
        );
        arr.js.is_indirect = true;
        type_table.insert(arr);
        type_table.insert(reference(ref_id, id, false));
    };
    mk_arr(ARG_LIST_FMT, ARG_LIST_FMT_REF, ARGUMENT, 9);
    mk_arr(ARG_LIST_WR, ARG_LIST_WR_REF, ARGUMENT, 2);
    let mut tpl1_desc = descriptor(
        TEMPLATE_FMT,
        TypeKind::Array {
            element: U8,
            length: tpl_fmt.len() as u64,
        },
        tpl_fmt.len() as u64,
    );
    tpl1_desc.js.is_indirect = true;
    type_table.insert(tpl1_desc);
    let mut tpl2_desc = descriptor(
        TEMPLATE_WR,
        TypeKind::Array {
            element: U8,
            length: tpl_wr.len() as u64,
        },
        tpl_wr.len() as u64,
    );
    tpl2_desc.js.is_indirect = true;
    type_table.insert(tpl2_desc);

    let mut items = ItemIndex::new();
    let mut machinery = |n: u64, link: &str, params: usize, standard: StandardOperation| {
        let id = ItemId {
            definition: def_id(n),
            instantiation: 0,
        };
        items.insert(
            id,
            ItemEntry {
                name: link.to_owned(),
                params,
                abi_known: true,
                result: None,
                arguments: Vec::new(),
                caller_location: false,
                caller_location_def: None,
                is_core: false,
                is_zst_closure_receiver: false,
                standard: Some(standard),
            },
        );
        Callee::Direct {
            item: id,
            link_name: interns.intern_str(link),
        }
    };
    let disp = machinery(
        501,
        "arg_display",
        1,
        StandardOperation::FormatArgumentNewDisplay,
    );
    let lhex = machinery(
        502,
        "arg_lhex",
        1,
        StandardOperation::FormatArgumentNewLowerHex,
    );
    let uhex = machinery(
        503,
        "arg_uhex",
        1,
        StandardOperation::FormatArgumentNewUpperHex,
    );
    let args_new = machinery(504, "args_new", 2, StandardOperation::FormatArgsNew);
    let fmt_to_s = machinery(
        505,
        "fmt_to_string",
        1,
        StandardOperation::FormatArgsToString,
    );
    let wr_fmt = machinery(506, "write_fmt", 2, StandardOperation::StringFmtWriteFmt);
    let unknown_arg = Callee::Direct {
        item: ItemId {
            definition: def_id(599),
            instantiation: 0,
        },
        link_name: interns.intern_str("arg_debug"),
    };
    let mk_call = |callee: &Callee, args: Vec<ValueId>, semantic: CallSemantic| OpKind::Call {
        callee: callee.clone(),
        function: None,
        args,
        track_caller: None,
        semantic,
        host: None,
    };

    // Function 1: `fmt_all(i, hx, ux, f, wrap, s_ref, st, ch, b) -> String`
    let fmt_item = ItemId {
        definition: def_id(600),
        instantiation: 0,
    };
    let mut b1 = FuncBuilder::new(
        fmt_item,
        sig(
            vec![
                I32,
                U32,
                U32,
                F64,
                WRAPPER_REF,
                STRING_REF,
                STR_REF,
                CHAR,
                BOOL,
            ],
            STRING,
        ),
    );
    let e1 = FuncBuilder::ENTRY;
    let p_i = b1.param(I32, ORIGIN);
    let p_hx = b1.param(U32, ORIGIN);
    let p_ux = b1.param(U32, ORIGIN);
    let p_f = b1.param(F64, ORIGIN);
    let p_wrap = b1.param(WRAPPER_REF, ORIGIN);
    let p_sref = b1.param(STRING_REF, ORIGIN);
    let p_st = b1.param(STR_REF, ORIGIN);
    let p_ch = b1.param(CHAR, ORIGIN);
    let p_b = b1.param(BOOL, ORIGIN);

    let borrow_cell = |fb: &mut FuncBuilder, v: ValueId, ty: TypeId, ref_ty: TypeId| {
        let cell = fb.cell(ty, true, ORIGIN);
        fb.effect(e1, OpKind::CellAlloc(cell), ORIGIN);
        fb.effect(e1, OpKind::CellStore { cell, v }, ORIGIN);
        fb.op(
            e1,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path: vec![],
                mutable: false,
            },
            ref_ty,
            ORIGIN,
        )
    };
    let r_i = borrow_cell(&mut b1, p_i, I32, I32_REF);
    let r_hx = borrow_cell(&mut b1, p_hx, U32, U32_REF);
    let r_ux = borrow_cell(&mut b1, p_ux, U32, U32_REF);
    let r_f = borrow_cell(&mut b1, p_f, F64, F64_REF);
    // `String` behind `Proj::Field` (`&(*wrap).label`).
    let r_wrap_str = b1.op(
        e1,
        OpKind::BorrowView {
            root: Root::Value(p_wrap),
            path: vec![
                Proj::Deref,
                Proj::Field {
                    field: 0,
                    ty: STRING,
                },
            ],
            mutable: false,
        },
        STRING_REF,
        ORIGIN,
    );
    // Direct `&String` parameter without an intermediate `BorrowView`.
    let r_direct_s = p_sref;
    let r_st = borrow_cell(&mut b1, p_st, STR_REF, PIECES_REF);
    let r_ch = borrow_cell(&mut b1, p_ch, CHAR, CHAR_REF);
    let r_b = borrow_cell(&mut b1, p_b, BOOL, BOOL_REF);

    let elems = vec![
        b1.op(
            e1,
            mk_call(&disp, vec![r_i], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&lhex, vec![r_hx], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&uhex, vec![r_ux], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&disp, vec![r_f], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&disp, vec![r_wrap_str], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&disp, vec![r_direct_s], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&disp, vec![r_st], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&disp, vec![r_ch], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
        b1.op(
            e1,
            mk_call(&disp, vec![r_b], CallSemantic::Ordinary),
            ARGUMENT,
            ORIGIN,
        ),
    ];
    let list1 = b1.op(
        e1,
        OpKind::Aggregate {
            ty: ARG_LIST_FMT,
            variant: 0,
            active_field: None,
            fields: elems,
        },
        ARG_LIST_FMT,
        ORIGIN,
    );
    let list1_ref = b1.op(
        e1,
        OpKind::BorrowView {
            root: Root::Value(list1),
            path: vec![],
            mutable: false,
        },
        ARG_LIST_FMT_REF,
        ORIGIN,
    );
    let tpl1_v = b1.op(
        e1,
        OpKind::Const(ConstTree::Aggregate(
            tpl_fmt
                .iter()
                .map(|&byte| ConstTree::Scalar {
                    bits: u128::from(byte),
                    bytes: 1,
                })
                .collect(),
        )),
        TEMPLATE_FMT,
        ORIGIN,
    );
    let args1 = b1.op(
        e1,
        mk_call(&args_new, vec![tpl1_v, list1_ref], CallSemantic::Ordinary),
        ARGUMENTS,
        ORIGIN,
    );
    let out1 = b1.op(
        e1,
        mk_call(&fmt_to_s, vec![args1], CallSemantic::Ordinary),
        STRING,
        ORIGIN,
    );
    b1.terminate(e1, RTerm::Return(Some(out1)));
    let func_fmt = b1.finish();

    // Function 2: `write_int_tag(out: &mut String, x: f64, ch: char) -> Result<(), fmt::Error>`
    // with `EnumRepr::IntTag { fieldless: true }` AND an unrelated unproven `Arguments::new`
    // call in a dead/conditional branch to verify per-site universal commit!
    let wr_item = ItemId {
        definition: def_id(601),
        instantiation: 0,
    };
    let mut b2 = FuncBuilder::new(
        wr_item,
        sig(vec![STRING_MUT, F64, CHAR, BOOL], FMT_RESULT_INT),
    );
    let e2 = FuncBuilder::ENTRY;
    let w_out = b2.param(STRING_MUT, ORIGIN);
    let w_f = b2.param(F64, ORIGIN);
    let w_ch = b2.param(CHAR, ORIGIN);
    let w_flag = b2.param(BOOL, ORIGIN);
    let bb_proven = b2.block();
    let bb_unproven = b2.block();
    b2.terminate(
        e2,
        RTerm::Switch {
            scrut: w_flag,
            scrut_ty: BOOL,
            cases: vec![(1, bb_proven, Vec::new())],
            default: (bb_unproven, Vec::new()),
        },
    );
    // Proven arm: `write!(out, "λ={:.5}:{}\n", w_f, w_ch)`
    let c_f = b2.cell(F64, true, ORIGIN);
    let c_ch = b2.cell(CHAR, true, ORIGIN);
    b2.effect(bb_proven, OpKind::CellAlloc(c_f), ORIGIN);
    b2.effect(bb_proven, OpKind::CellStore { cell: c_f, v: w_f }, ORIGIN);
    b2.effect(bb_proven, OpKind::CellAlloc(c_ch), ORIGIN);
    b2.effect(
        bb_proven,
        OpKind::CellStore {
            cell: c_ch,
            v: w_ch,
        },
        ORIGIN,
    );
    let rf2 = b2.op(
        bb_proven,
        OpKind::BorrowView {
            root: Root::Cell(c_f),
            path: vec![],
            mutable: false,
        },
        F64_REF,
        ORIGIN,
    );
    let rch2 = b2.op(
        bb_proven,
        OpKind::BorrowView {
            root: Root::Cell(c_ch),
            path: vec![],
            mutable: false,
        },
        CHAR_REF,
        ORIGIN,
    );
    let a_f2 = b2.op(
        bb_proven,
        mk_call(&disp, vec![rf2], CallSemantic::Ordinary),
        ARGUMENT,
        ORIGIN,
    );
    let a_ch2 = b2.op(
        bb_proven,
        mk_call(&disp, vec![rch2], CallSemantic::Ordinary),
        ARGUMENT,
        ORIGIN,
    );
    let list2 = b2.op(
        bb_proven,
        OpKind::Aggregate {
            ty: ARG_LIST_WR,
            variant: 0,
            active_field: None,
            fields: vec![a_f2, a_ch2],
        },
        ARG_LIST_WR,
        ORIGIN,
    );
    let list2_ref = b2.op(
        bb_proven,
        OpKind::BorrowView {
            root: Root::Value(list2),
            path: vec![],
            mutable: false,
        },
        ARG_LIST_WR_REF,
        ORIGIN,
    );
    let tpl2_v = b2.op(
        bb_proven,
        OpKind::Const(ConstTree::Aggregate(
            tpl_wr
                .iter()
                .map(|&byte| ConstTree::Scalar {
                    bits: u128::from(byte),
                    bytes: 1,
                })
                .collect(),
        )),
        TEMPLATE_WR,
        ORIGIN,
    );
    let args2 = b2.op(
        bb_proven,
        mk_call(&args_new, vec![tpl2_v, list2_ref], CallSemantic::Ordinary),
        ARGUMENTS,
        ORIGIN,
    );
    let wr_res = b2.op(
        bb_proven,
        mk_call(
            &wr_fmt,
            vec![w_out, args2],
            CallSemantic::Standard(StandardOperation::StringFmtWriteFmt),
        ),
        FMT_RESULT_INT,
        ORIGIN,
    );
    b2.terminate(bb_proven, RTerm::Return(Some(wr_res)));

    // Unproven arm: calls an unrecognized `arg_debug` + `args_new` + `write_fmt`.
    let dbg_arg = b2.op(
        bb_unproven,
        mk_call(&unknown_arg, vec![w_f], CallSemantic::Ordinary),
        ARGUMENT,
        ORIGIN,
    );
    let list_u = b2.op(
        bb_unproven,
        OpKind::Aggregate {
            ty: ARG_LIST_WR,
            variant: 0,
            active_field: None,
            fields: vec![dbg_arg, dbg_arg],
        },
        ARG_LIST_WR,
        ORIGIN,
    );
    let list_u_ref = b2.op(
        bb_unproven,
        OpKind::BorrowView {
            root: Root::Value(list_u),
            path: vec![],
            mutable: false,
        },
        ARG_LIST_WR_REF,
        ORIGIN,
    );
    let tpl_u = b2.op(
        bb_unproven,
        OpKind::Const(ConstTree::Aggregate(
            tpl_wr
                .iter()
                .map(|&byte| ConstTree::Scalar {
                    bits: u128::from(byte),
                    bytes: 1,
                })
                .collect(),
        )),
        TEMPLATE_WR,
        ORIGIN,
    );
    let args_u = b2.op(
        bb_unproven,
        mk_call(&args_new, vec![tpl_u, list_u_ref], CallSemantic::Ordinary),
        ARGUMENTS,
        ORIGIN,
    );
    let wr_u = b2.op(
        bb_unproven,
        mk_call(
            &wr_fmt,
            vec![w_out, args_u],
            CallSemantic::Standard(StandardOperation::StringFmtWriteFmt),
        ),
        FMT_RESULT_INT,
        ORIGIN,
    );
    b2.terminate(bb_unproven, RTerm::Return(Some(wr_u)));
    let func_wr = b2.finish();

    // Put `type_table` into `interns.types` so `LinkItem::jir::interns()` has the full TypeTable.
    interns.types = type_table;
    let make_link_item = |name: &str, func: &crate::jir::repr::Func| {
        let mut item = LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: String::new(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: ItemLinkage::internal(),
            debug_path: name.to_owned(),
            crate_name: "test".to_owned(),
            order: None,
            map: Default::default(),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        };
        item.jir.use_interns(interns.clone());
        item.jir.body = Some(Body {
            item: func.item,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: func.sig.clone(),
            locals: Vec::new(),
            blocks: Vec::new(),
        });
        item.jir.summary = Some(ItemSummary {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: Some(func.item),
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
        });
        item
    };

    let n_fmt = JsName::new("fmt_all".to_owned());
    let n_wr = JsName::new("write_int_tag".to_owned());
    let mut link_table = BTreeMap::new();
    link_table.insert(n_fmt.clone(), make_link_item("fmt_all", &func_fmt));
    link_table.insert(n_wr.clone(), make_link_item("write_int_tag", &func_wr));
    let mut funcs = BTreeMap::from([(n_fmt.clone(), Ok(func_fmt)), (n_wr.clone(), Ok(func_wr))]);
    let live: BTreeSet<JsName> = [n_fmt.clone(), n_wr.clone()].into_iter().collect();
    let skip = BTreeSet::new();

    let touched = crate::lower::semantic_ops::fmt_kernel::commit_universal_native_string_sites(
        &link_table,
        &mut funcs,
        &live,
        &skip,
        &items,
    );
    assert_eq!(
        touched, live,
        "both functions should commit proven universal format sites"
    );

    let heap = HeapPlan {
        native_string_enabled: true,
        ..HeapPlan::default()
    };
    let opts = Opts::default();
    let lower_one = |name: &JsName| {
        let li = &link_table[name];
        let func = funcs[name].as_ref().unwrap();
        let li_interns = li.jir.interns().unwrap();
        let input = LowerInput {
            name,
            summary: li.jir.summary.as_ref().unwrap(),
            body: li.jir.body.as_ref().unwrap(),
            origins: None,
            interns: li_interns,
            table: &li_interns.types,
            opts: &opts,
            items: &items,
            drop_glue: crate::lower::js::host::EMPTY_DROP_GLUE,
            vtables: &crate::lower::js::unsize::EMPTY_VTABLES,
            spills: &[],
            abi: None,
            final_lowering: false,
            call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
            bigints: None,
            byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
            heap: Some(&heap),
            nested_clone: crate::lower::NestedClone::default(),
        };
        let lowered = lower_func(&input, func)
            .unwrap_or_else(|d| panic!("universal fmt_kernel declined `{name}`: {d}"));
        jsast::stmt_to_string(&lowered.decl)
    };

    let js_fmt = lower_one(&n_fmt);
    let js_wr = lower_one(&n_wr);
    for forbidden in [
        "arg_display",
        "arg_lhex",
        "arg_uhex",
        "args_new",
        "fmt_to_string",
        "native_str_wf",
    ] {
        assert!(
            !js_fmt.contains(forbidden),
            "`fmt_all` should not retain `{forbidden}`:\n{js_fmt}"
        );
    }
    assert!(
        js_fmt.contains("fromCharCode") && js_fmt.contains("bstr_char"),
        "`fmt_all` should emit fast ASCII `fromCharCode` + non-ASCII `bstr_char`:\n{js_fmt}"
    );
    // In `write_int_tag`, the unproven branch keeps `arg_debug`/`write_fmt`, while the proven
    // branch lowers inline to `out.s += ...` and returns `0` (IntTag Ok).
    assert!(
        js_wr.contains("arg_debug") && js_wr.contains(".s +="),
        "`write_int_tag` should commit the proven site and keep the unproven site:\n{js_wr}"
    );

    run_in_node(
        &[js_fmt, js_wr],
        r#"
const assert = require('node:assert/strict');
function toBstr(utf16) {
  const bytes = new TextEncoder().encode(utf16);
  let out = '';
  for (let i = 0; i < bytes.length; i++) out += String.fromCharCode(bytes[i]);
  return out;
}
function fromBstr(bstr) {
  const bytes = new Uint8Array(bstr.length);
  for (let i = 0; i < bstr.length; i++) bytes[i] = bstr.charCodeAt(i);
  return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
}

// 1) `format!` with ASCII char 'A' (65) and non-ASCII UTF-8 strings/template literals.
const r1 = fmt_all(
  42,
  0xdeadbeef,
  0x0000cafe,
  3.14159265,
  { label: { s: toBstr('héllo'), cap: 6 } },
  { s: toBstr('wörld'), cap: 6 },
  toBstr('naïve'),
  65,
  true,
);
const exp1 = 'café=+0042 hex=deadbeef HEX=0x00CAFE f=3.14159 s=héllo ds=wörld str=naïve ch=A b=true 🦀';
assert.strictEqual(r1.s, toBstr(exp1));
assert.strictEqual(fromBstr(r1.s), exp1);
assert.strictEqual(r1.cap, r1.s.length);

// 2) `format!` with non-ASCII chars ('é' = 0xe9, '🦀' = 0x1f980) and negative int / false.
const r2 = fmt_all(
  -7,
  0x1a,
  0x2b,
  -0.5,
  { label: { s: '', cap: 0 } },
  { s: toBstr('π'), cap: 2 },
  '',
  0x1f980,
  false,
);
const exp2 = 'café=-0007 hex=0000001a HEX=0x00002B f=-0.50000 s= ds=π str= ch=🦀 b=false 🦀';
assert.strictEqual(r2.s, toBstr(exp2));
assert.strictEqual(fromBstr(r2.s), exp2);
assert.strictEqual(r2.cap, r2.s.length);

// 3) `write!` into `{ s, cap }` with `EnumRepr::IntTag { fieldless: true }` returning `0`.
const out = { s: toBstr('init|'), cap: 5 };
const ret1 = write_int_tag(out, 1.25, 0x41, true);
assert.strictEqual(ret1, 0, 'IntTag Ok(()) must return 0');
const ret2 = write_int_tag(out, 2.7182818, 0xe9, true);
assert.strictEqual(ret2, 0, 'IntTag Ok(()) must return 0 on second write');
const expOut = 'init|λ=1.25000:A\nλ=2.71828:é\n';
assert.strictEqual(out.s, toBstr(expOut));
assert.strictEqual(fromBstr(out.s), expOut);
"#,
    );
}
