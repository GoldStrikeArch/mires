use super::convert::{make_link_item, ConvCx, ConvTarget, Mode};
use super::opt::{
    elide_ascii_only_bstr_in, elide_known_ascii_bstr_out, elide_proven_bstr_sub,
    elide_readonly_bstr_bytes,
};
use super::*;

use crate::item::JsName;
use crate::jir::interns::Interns;
use crate::jir::types::{
    FieldDescriptor, FieldKey, NominalForm, PointerForm, RepresentationCandidate, TypeDescriptor,
    TypeId, TypeKind, TypeTable, VariantDescriptor,
};
use crate::jsast::{self, Expr, Stmt};

fn rt(member: &str, args: Vec<Expr>) -> Expr {
    jsast::rt_call(member, args)
}

#[test]
fn a_literal_is_the_string_of_its_bytes() {
    assert_eq!(byte_string("aé😀"), "a\u{c3}\u{a9}\u{f0}\u{9f}\u{98}\u{80}");
}

#[test]
fn the_rewrite_swaps_helpers_and_folds_lengths() {
    let mut stmt = jsast::ret(jsast::binary(
        jsast::BinOp::Add,
        rt(
            "str_len",
            vec![rt(
                "bytes_str",
                vec![jsast::id("b"), jsast::num(0), jsast::num(2)],
            )],
        ),
        rt("str_len", vec![rt("native_str_wf", vec![jsast::id("s")])]),
    ));
    rewrite(&mut stmt);
    let printed = jsast::program_to_string(std::slice::from_ref(&stmt));
    assert!(
        printed.contains("__rt.bstr_of(b, 0, 2).length"),
        "{printed}"
    );
    assert!(printed.contains("s.length"), "{printed}");
    assert_eq!(leftover_helper(&printed), None, "{printed}");
}

#[test]
fn leftovers_are_found_by_whole_name() {
    assert_eq!(leftover_helper("__rt.str_bytes(s)"), Some("str_bytes"));
    assert_eq!(
        leftover_helper("__rt.str_bytes_mut(s)"),
        Some("str_bytes_mut")
    );
    assert_eq!(leftover_helper("__rt.bstr_bytes(s); __rt.str_lenx"), None);
}

fn make_desc(id: u128, kind: TypeKind, zst: bool) -> TypeDescriptor {
    use crate::jir::types::{JsFacts, LayoutFacts, TypeMeta};
    let mut d = TypeDescriptor {
        id: TypeId(id),
        name: None,
        kind,
        layout: LayoutFacts {
            size: Some(if zst { 0 } else { 8 }),
            unsized_prefix: None,
            align: Some(1),
            sized: true,
            zst,
            one_zst: zst,
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
            peeled: TypeId(id),
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        },
    };
    if zst {
        d.js.is_indirect = true;
    }
    d
}

#[test]
fn reaches_str_walks_adt_args_closures_and_coroutines_while_excluding_string() {
    use crate::jir::semantic::DefinitionId;
    let mut interns = Interns::new();
    let def = DefinitionId {
        crate_id: 1,
        local_hash: 1,
    };
    let str_id = TypeId(1);
    let ref_str_id = TypeId(2);
    let u8_id = TypeId(3);
    let phantom_str_id = TypeId(4);
    let string_raw_id = TypeId(5);
    let closure_id = TypeId(6);
    let coro_id = TypeId(7);
    let coro_closure_id = TypeId(8);
    let zst_arr_id = TypeId(9);

    let mut ref_str = make_desc(
        ref_str_id.0,
        TypeKind::Reference {
            pointee: str_id,
            mutable: false,
        },
        false,
    );
    ref_str.js.pointer_form = Some(PointerForm::Str);

    let mut phantom_str = make_desc(
        phantom_str_id.0,
        TypeKind::Adt {
            def,
            args: vec![ref_str_id],
            form: NominalForm::Struct,
            variants: vec![VariantDescriptor {
                name: interns.intern_str("PhantomData"),
                fields: Vec::new(),
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        true,
    );
    phantom_str.js.fields = Vec::new();

    let string_adt = make_desc(
        string_raw_id.0,
        TypeKind::Adt {
            def,
            args: vec![u8_id],
            form: NominalForm::Struct,
            variants: vec![VariantDescriptor {
                name: interns.intern_str("String"),
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(interns.intern_str("buf")),
                    ty: u8_id,
                    offset: 0,
                }],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        false,
    );

    let closure_d = make_desc(
        closure_id.0,
        TypeKind::Closure {
            def,
            upvars: vec![ref_str_id],
            fn_pointer_shim: None,
        },
        false,
    );

    let coro_d = make_desc(
        coro_id.0,
        TypeKind::Coroutine {
            def,
            upvars: Vec::new(),
            saved_locals: vec![(0, ref_str_id)],
            states: Vec::new(),
            variants: Vec::new(),
        },
        false,
    );

    let coro_closure_d = make_desc(
        coro_closure_id.0,
        TypeKind::CoroutineClosure {
            def,
            upvars: vec![ref_str_id],
        },
        false,
    );

    let zst_arr_d = make_desc(
        zst_arr_id.0,
        TypeKind::Array {
            element: ref_str_id,
            length: 0,
        },
        true,
    );

    let table = TypeTable::from(vec![
        make_desc(str_id.0, TypeKind::Str, false),
        ref_str,
        make_desc(
            u8_id.0,
            TypeKind::Integer {
                signed: false,
                bits: 8,
            },
            false,
        ),
        phantom_str,
        string_adt,
        closure_d,
        coro_d,
        coro_closure_d,
        zst_arr_d,
    ]);

    assert!(reaches_str(&table, phantom_str_id));
    assert!(!reaches_str(&table, string_raw_id));
    assert!(reaches_str(&table, closure_id));
    assert!(reaches_str(&table, coro_id));
    assert!(reaches_str(&table, coro_closure_id));
    assert!(reaches_str(&table, zst_arr_id));

    // ZST array `[&str; 0]` short-circuits without `.map()`.
    let passed = host_convert(&table, &interns, zst_arr_id, jsast::id("z"), Crossing::Out)
        .expect("zst array must pass");
    assert_eq!(passed, jsast::id("z"));

    // Coroutine value in `js!` is refused.
    assert!(host_convert(&table, &interns, coro_id, jsast::id("g"), Crossing::Out).is_err());

    // Closure value emits a deduplicated helper function call (not inline Object.assign).
    let clo_call = host_convert(&table, &interns, closure_id, jsast::id("c"), Crossing::Out)
        .expect("closure converts");
    assert!(
        matches!(clo_call, Expr::Call(ref f, _) if matches!(&**f, Expr::Ident(n) if n.starts_with("$bstr$out$"))),
        "{clo_call:?}"
    );
}

#[test]
fn depth_cap_allows_64_and_refuses_65_and_refuses_raw_and_union() {
    use crate::jir::semantic::DefinitionId;
    let mut interns = Interns::new();
    let def = DefinitionId {
        crate_id: 2,
        local_hash: 1,
    };
    let str_id = TypeId(1);
    let ref_str_id = TypeId(2);
    let mut ref_str = make_desc(
        ref_str_id.0,
        TypeKind::Reference {
            pointee: str_id,
            mutable: false,
        },
        false,
    );
    ref_str.js.pointer_form = Some(PointerForm::Str);

    let mut descs = vec![make_desc(str_id.0, TypeKind::Str, false), ref_str];
    let mut cur = ref_str_id;
    let f_name = interns.intern_str("_0");
    let s_name = interns.intern_str("Wrap");
    for lvl in 1..=66u128 {
        let id = TypeId(100 + lvl);
        let d = make_desc(
            id.0,
            TypeKind::Adt {
                def,
                args: Vec::new(),
                form: NominalForm::Struct,
                variants: vec![VariantDescriptor {
                    name: s_name,
                    fields: vec![FieldDescriptor {
                        key: FieldKey::Name(f_name),
                        ty: cur,
                        offset: 0,
                    }],
                    logical_discr: 0,
                    physical_tag: None,
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                }],
                niche: None,
            },
            false,
        );
        descs.push(d);
        cur = id;
    }
    let d63 = TypeId(163);
    let d64 = TypeId(164);
    let d65 = TypeId(165);
    let raw_pair = TypeId(200);
    let tup = TypeId(201);
    let uni = TypeId(202);
    let u32_id = TypeId(203);

    descs.push(make_desc(
        u32_id.0,
        TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        false,
    ));
    let mut tup_d = make_desc(tup.0, TypeKind::Tuple(vec![ref_str_id, u32_id]), false);
    tup_d.js.fields = vec![
        FieldDescriptor {
            key: FieldKey::Index(0),
            ty: ref_str_id,
            offset: 0,
        },
        FieldDescriptor {
            key: FieldKey::Index(1),
            ty: u32_id,
            offset: 4,
        },
    ];
    descs.push(tup_d);
    descs.push(make_desc(
        raw_pair.0,
        TypeKind::RawPointer {
            pointee: tup,
            mutable: false,
        },
        false,
    ));
    descs.push(make_desc(
        uni.0,
        TypeKind::Adt {
            def,
            args: Vec::new(),
            form: NominalForm::Union,
            variants: vec![VariantDescriptor {
                name: interns.intern_str("U"),
                fields: vec![
                    FieldDescriptor {
                        key: FieldKey::Name(interns.intern_str("a")),
                        ty: ref_str_id,
                        offset: 0,
                    },
                    FieldDescriptor {
                        key: FieldKey::Name(interns.intern_str("n")),
                        ty: u32_id,
                        offset: 0,
                    },
                ],
                logical_discr: 0,
                physical_tag: None,
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        false,
    ));

    let table = TypeTable::from(descs);
    assert!(host_convert(&table, &interns, d63, jsast::id("v"), Crossing::Out).is_ok());
    assert!(host_convert(&table, &interns, d64, jsast::id("v"), Crossing::Out).is_ok());
    assert!(host_convert(&table, &interns, d65, jsast::id("v"), Crossing::Out).is_err());
    assert!(host_convert(&table, &interns, raw_pair, jsast::id("p"), Crossing::Out).is_err());
    assert!(host_convert(&table, &interns, uni, jsast::id("u"), Crossing::Out).is_err());
}

#[test]
fn slot_pointer_and_slice_pointer_emit_distinct_descriptors() {
    let interns = Interns::default();
    let str_id = TypeId(1);
    let ref_str_id = TypeId(2);
    let slot_ref_id = TypeId(3);
    let slice_id = TypeId(4);
    let ref_slice_id = TypeId(5);

    let mut ref_str = make_desc(
        ref_str_id.0,
        TypeKind::Reference {
            pointee: str_id,
            mutable: false,
        },
        false,
    );
    ref_str.js.pointer_form = Some(PointerForm::Str);

    let mut slot_ref = make_desc(
        slot_ref_id.0,
        TypeKind::Reference {
            pointee: ref_str_id,
            mutable: false,
        },
        false,
    );
    slot_ref.js.pointer_form = Some(PointerForm::Slot);

    let slice_desc = make_desc(slice_id.0, TypeKind::Slice(ref_str_id), false);

    let mut ref_slice = make_desc(
        ref_slice_id.0,
        TypeKind::Reference {
            pointee: slice_id,
            mutable: false,
        },
        false,
    );
    ref_slice.js.pointer_form = Some(PointerForm::FatSlice);

    let table = TypeTable::from(vec![
        make_desc(str_id.0, TypeKind::Str, false),
        ref_str,
        slot_ref,
        slice_desc,
        ref_slice,
    ]);

    let mut cx = ConvCx::new(&table, &interns, false);
    let slot_stmt = cx
        .build_decl(
            &JsName::new("slot_fn".to_owned()),
            ConvTarget::Type(slot_ref_id, Mode::Val),
            Crossing::Out,
        )
        .expect("slot decl");
    let slot_js = make_link_item(JsName::new("slot_fn".to_owned()), slot_stmt).js;
    assert!(
        slot_js.contains("k: \"slot\"") || slot_js.contains("k:\"slot\""),
        "slot JS: {slot_js}"
    );
    assert!(!slot_js.contains("k: \"slice\"") && !slot_js.contains("k:\"slice\""));

    let slice_stmt = cx
        .build_decl(
            &JsName::new("slice_fn".to_owned()),
            ConvTarget::Type(ref_slice_id, Mode::Val),
            Crossing::Out,
        )
        .expect("slice decl");
    let slice_js = make_link_item(JsName::new("slice_fn".to_owned()), slice_stmt).js;
    assert!(
        slice_js.contains("k: \"slice\"") || slice_js.contains("k:\"slice\""),
        "slice JS: {slice_js}"
    );
    assert!(!slice_js.contains("k: \"slot\"") && !slice_js.contains("k:\"slot\""));
}

#[test]
fn mut_str_pointers_are_live_byte_slices_not_bstr() {
    let str_id = TypeId(1);
    let ref_str_id = TypeId(2);
    let mut_str_id = TypeId(3);
    let const_ptr_id = TypeId(4);
    let mut_ptr_id = TypeId(5);
    let tuple_mut_id = TypeId(6);

    let mut ref_str = make_desc(
        ref_str_id.0,
        TypeKind::Reference {
            pointee: str_id,
            mutable: false,
        },
        false,
    );
    ref_str.js.pointer_form = Some(PointerForm::Str);

    let mut mut_str = make_desc(
        mut_str_id.0,
        TypeKind::Reference {
            pointee: str_id,
            mutable: true,
        },
        false,
    );
    mut_str.js.pointer_form = Some(PointerForm::FatSlice);

    let mut const_ptr = make_desc(
        const_ptr_id.0,
        TypeKind::RawPointer {
            pointee: str_id,
            mutable: false,
        },
        false,
    );
    const_ptr.js.pointer_form = Some(PointerForm::FatSlice);

    let mut mut_ptr = make_desc(
        mut_ptr_id.0,
        TypeKind::RawPointer {
            pointee: str_id,
            mutable: true,
        },
        false,
    );
    mut_ptr.js.pointer_form = Some(PointerForm::FatSlice);

    let tuple_mut = make_desc(tuple_mut_id.0, TypeKind::Tuple(vec![mut_str_id]), false);
    let box_str_id = TypeId(7);
    let mut box_str = make_desc(
        box_str_id.0,
        TypeKind::Adt {
            def: crate::jir::DefinitionId {
                crate_id: 1,
                local_hash: 7,
            },
            args: vec![str_id],
            form: NominalForm::Struct,
            variants: Vec::new(),
            niche: None,
        },
        false,
    );
    box_str.meta.candidate = RepresentationCandidate::Box;
    box_str.js.peeled = const_ptr_id;
    let tuple_box_id = TypeId(8);
    let tuple_box = make_desc(tuple_box_id.0, TypeKind::Tuple(vec![box_str_id]), false);

    let table = TypeTable::from(vec![
        make_desc(str_id.0, TypeKind::Str, false),
        ref_str,
        mut_str,
        const_ptr,
        mut_ptr,
        tuple_mut,
        box_str,
        tuple_box,
    ]);

    assert!(is_str_pointer(&table, ref_str_id));
    assert!(!is_str_pointer(&table, const_ptr_id));
    assert!(!is_str_pointer(&table, mut_str_id));
    assert!(!is_str_pointer(&table, mut_ptr_id));
    assert!(!is_str_pointer(&table, box_str_id));

    assert!(reaches_str(&table, ref_str_id));
    assert!(!reaches_str(&table, const_ptr_id));
    assert!(!reaches_str(&table, mut_str_id));
    assert!(!reaches_str(&table, mut_ptr_id));
    assert!(!reaches_str(&table, tuple_mut_id));
    assert!(!reaches_str(&table, box_str_id));
    assert!(!reaches_str(&table, tuple_box_id));
}

#[test]
fn rewrites_str_case_helpers_and_catches_leftovers() {
    let mut stmt = jsast::ret(jsast::binary(
        jsast::BinOp::Add,
        rt("str_to_lowercase", vec![jsast::id("a")]),
        rt("str_to_uppercase", vec![jsast::id("b")]),
    ));
    rewrite(&mut stmt);
    let printed = jsast::program_to_string(std::slice::from_ref(&stmt));
    assert!(printed.contains("__rt.bstr_to_lowercase(a)"), "{printed}");
    assert!(printed.contains("__rt.bstr_to_uppercase(b)"), "{printed}");
    assert_eq!(leftover_helper(&printed), None, "{printed}");
    assert_eq!(
        leftover_helper("__rt.str_to_lowercase(s)"),
        Some("str_to_lowercase")
    );
    assert_eq!(
        leftover_helper("__rt.str_to_uppercase(s)"),
        Some("str_to_uppercase")
    );
}

#[test]
fn folds_bstr_slice_roundtrip_and_elides_readonly_bstr_bytes() {
    // 1. `($t0 = __rt.sfp(__rt.bstr_bytes(s), b), __rt.bstr_of($t0.buf, $t0.off, $t0.len))` -> `s.slice(0, b)`.
    let mut roundtrip = jsast::ret(Expr::Seq(vec![
        Expr::Assign(
            Box::new(jsast::id("$t0")),
            Box::new(rt(
                "sfp",
                vec![rt("str_bytes", vec![jsast::id("s")]), jsast::id("b")],
            )),
        ),
        rt(
            "bytes_str",
            vec![
                Expr::Member(Box::new(jsast::id("$t0")), "buf".to_owned()),
                Expr::Member(Box::new(jsast::id("$t0")), "off".to_owned()),
                Expr::Member(Box::new(jsast::id("$t0")), "len".to_owned()),
            ],
        ),
    ]));
    rewrite(&mut roundtrip);
    let printed = jsast::program_to_string(std::slice::from_ref(&roundtrip));
    assert!(printed.contains("s.slice(0, b)"), "{printed}");
    assert!(!printed.contains("bstr_bytes"), "{printed}");

    // 2. Read-only `let bv; bv = __rt.bstr_bytes(s); return bv.buf[bv.off + i];` -> `bv = s; return bv.charCodeAt(i);`.
    let mut body = vec![
        Stmt::Let("bv".to_owned(), None),
        jsast::assign_stmt(jsast::id("bv"), rt("bstr_bytes", vec![jsast::id("s")])),
        Stmt::Return(Some(Expr::Index(
            Box::new(Expr::Member(Box::new(jsast::id("bv")), "buf".to_owned())),
            Box::new(Expr::Binary(
                jsast::BinOp::Add,
                Box::new(Expr::Member(Box::new(jsast::id("bv")), "off".to_owned())),
                Box::new(jsast::id("i")),
            )),
        ))),
    ];
    elide_readonly_bstr_bytes(&mut body);
    let printed_ro = jsast::program_to_string(&body);
    assert!(printed_ro.contains("bv = s;"), "{printed_ro}");
    assert!(printed_ro.contains("bv.charCodeAt(i)"), "{printed_ro}");
    assert!(!printed_ro.contains("bstr_bytes"), "{printed_ro}");
}

fn run_speed_passes(stmt: &mut Stmt) {
    rewrite(stmt);
    if let Stmt::FunctionDecl { body, .. } = stmt {
        elide_readonly_bstr_bytes(body);
        elide_ascii_only_bstr_in(body);
        let known_ascii = elide_known_ascii_bstr_out(body);
        elide_proven_bstr_sub(body, &known_ascii);
    }
}

#[test]
fn widened_elide_ascii_only_bstr_in_handles_methods_charcodeat_and_cond() {
    let with_speed = run_speed_passes;

    // Positive 1: ASCII startsWith / endsWith / includes / charCodeAt(0) / length === 0.
    let mut f_ok = Stmt::FunctionDecl {
        name: "f_ok".to_owned(),
        params: vec!["s".to_owned()],
        body: vec![
            Stmt::Let("a".to_owned(), Some(rt("bstr_in", vec![jsast::id("s")]))),
            jsast::ret(jsast::binary(
                jsast::BinOp::Or,
                jsast::binary(
                    jsast::BinOp::Or,
                    jsast::method_call(jsast::id("a"), "startsWith", vec![jsast::string("+")]),
                    jsast::method_call(jsast::id("a"), "endsWith", vec![jsast::string("-")]),
                ),
                jsast::binary(
                    jsast::BinOp::Or,
                    jsast::method_call(jsast::id("a"), "includes", vec![jsast::string("0")]),
                    jsast::binary(
                        jsast::BinOp::Or,
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            jsast::method_call(jsast::id("a"), "charCodeAt", vec![jsast::num(0)]),
                            jsast::num(45),
                        ),
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            jsast::member(jsast::id("a"), "length"),
                            jsast::num(0),
                        ),
                    ),
                ),
            )),
        ],
    };
    with_speed(&mut f_ok);
    let p_ok = jsast::stmt_to_string(&f_ok);
    assert!(!p_ok.contains("bstr_in"), "{p_ok}");

    // Positive 2: Cond arm with bstr_in + ASCII fallback literal, and inline bstr_in unwrap.
    let mut f_cond = Stmt::FunctionDecl {
        name: "f_cond".to_owned(),
        params: vec!["s".to_owned(), "flag".to_owned()],
        body: vec![
            Stmt::Let(
                "a".to_owned(),
                Some(jsast::cond(
                    jsast::id("flag"),
                    rt("bstr_in", vec![jsast::id("s")]),
                    jsast::string("0"),
                )),
            ),
            jsast::ret(jsast::binary(
                jsast::BinOp::Or,
                jsast::binary(jsast::BinOp::StrictEq, jsast::id("a"), jsast::string("1")),
                jsast::binary(
                    jsast::BinOp::Or,
                    jsast::method_call(
                        rt("bstr_in", vec![jsast::id("s")]),
                        "startsWith",
                        vec![jsast::string("abc")],
                    ),
                    jsast::binary(
                        jsast::BinOp::StrictNe,
                        jsast::member(rt("bstr_in", vec![jsast::id("s")]), "length"),
                        jsast::num(0),
                    ),
                ),
            )),
        ],
    };
    with_speed(&mut f_cond);
    let p_cond = jsast::stmt_to_string(&f_cond);
    assert!(!p_cond.contains("bstr_in"), "{p_cond}");

    // Negative cases: length === 1, charCodeAt(1), non-ASCII literal, charCodeAt(0) === 128.
    let negatives = [
        jsast::binary(
            jsast::BinOp::StrictEq,
            jsast::member(jsast::id("a"), "length"),
            jsast::num(1),
        ),
        jsast::binary(
            jsast::BinOp::StrictEq,
            jsast::method_call(jsast::id("a"), "charCodeAt", vec![jsast::num(1)]),
            jsast::num(45),
        ),
        jsast::method_call(
            jsast::id("a"),
            "startsWith",
            vec![jsast::string("\u{00c3}\u{00a9}")],
        ),
        jsast::binary(
            jsast::BinOp::StrictEq,
            jsast::method_call(jsast::id("a"), "charCodeAt", vec![jsast::num(0)]),
            jsast::num(128),
        ),
    ];
    for (idx, ret_expr) in negatives.into_iter().enumerate() {
        let mut f_bad = Stmt::FunctionDecl {
            name: format!("f_bad_{idx}"),
            params: vec!["s".to_owned()],
            body: vec![
                Stmt::Let("a".to_owned(), Some(rt("bstr_in", vec![jsast::id("s")]))),
                jsast::ret(ret_expr),
            ],
        };
        with_speed(&mut f_bad);
        let p_bad = jsast::stmt_to_string(&f_bad);
        assert!(
            p_bad.contains("bstr_in"),
            "negative case {idx} must keep bstr_in: {p_bad}"
        );
    }
}

#[test]
fn elide_known_ascii_bstr_out_tracks_scalar_and_native_string_objects() {
    let with_speed = run_speed_passes;

    // Positive 1: scalar string built from literals, bounded fromCharCode, toString, slice.
    let mut f_scalar = Stmt::FunctionDecl {
        name: "fmt_scalar".to_owned(),
        params: vec!["d".to_owned(), "n".to_owned()],
        body: vec![
            Stmt::Let("s".to_owned(), Some(jsast::string(""))),
            jsast::expr_stmt(jsast::compound_assign(
                jsast::CompoundOp::Add,
                jsast::id("s"),
                jsast::string("-"),
            )),
            jsast::expr_stmt(jsast::compound_assign(
                jsast::CompoundOp::Add,
                jsast::id("s"),
                jsast::method_call(
                    jsast::id("String"),
                    "fromCharCode",
                    vec![jsast::binary(
                        jsast::BinOp::UShr,
                        jsast::binary(
                            jsast::BinOp::Add,
                            jsast::num(48),
                            jsast::binary(jsast::BinOp::BitAnd, jsast::id("d"), jsast::num(7)),
                        ),
                        jsast::num(0),
                    )],
                ),
            )),
            jsast::assign_stmt(
                jsast::id("s"),
                jsast::binary(
                    jsast::BinOp::Add,
                    jsast::id("s"),
                    jsast::method_call(
                        jsast::binary(jsast::BinOp::BitOr, jsast::id("n"), jsast::num(0)),
                        "toString",
                        Vec::new(),
                    ),
                ),
            ),
            Stmt::Let(
                "t".to_owned(),
                Some(jsast::method_call(
                    jsast::id("s"),
                    "slice",
                    vec![jsast::num(0), jsast::num(2)],
                )),
            ),
            jsast::ret(jsast::array(vec![
                rt("bstr_out", vec![jsast::id("s")]),
                rt("_bstr_is_ascii", vec![jsast::id("s")]),
                rt("bstr_to_lowercase", vec![jsast::id("t")]),
            ])),
        ],
    };
    with_speed(&mut f_scalar);
    let p_scalar = jsast::stmt_to_string(&f_scalar);
    assert!(!p_scalar.contains("bstr_out"), "{p_scalar}");
    assert!(!p_scalar.contains("_bstr_is_ascii"), "{p_scalar}");
    assert!(!p_scalar.contains("bstr_to_lowercase"), "{p_scalar}");
    assert!(p_scalar.contains("t.toLowerCase()"), "{p_scalar}");
    assert!(p_scalar.contains("[s, true,"), "{p_scalar}");

    // Positive 2: local native `{ s, cap }` String object mutated in place.
    let mut f_obj = Stmt::FunctionDecl {
        name: "fmt_obj".to_owned(),
        params: vec!["d".to_owned()],
        body: vec![
            Stmt::Let(
                "o".to_owned(),
                Some(jsast::object_of(vec![
                    ("s", jsast::string("")),
                    ("cap", jsast::num(0)),
                ])),
            ),
            jsast::expr_stmt(jsast::compound_assign(
                jsast::CompoundOp::Add,
                jsast::member(jsast::id("o"), "s"),
                jsast::string("0."),
            )),
            jsast::expr_stmt(jsast::compound_assign(
                jsast::CompoundOp::Add,
                jsast::member(jsast::id("o"), "s"),
                jsast::method_call(
                    jsast::id("String"),
                    "fromCharCode",
                    vec![jsast::binary(
                        jsast::BinOp::Add,
                        jsast::num(48),
                        jsast::binary(jsast::BinOp::Rem, jsast::id("d"), jsast::num(10)),
                    )],
                ),
            )),
            jsast::ret(rt("bstr_out", vec![jsast::member(jsast::id("o"), "s")])),
        ],
    };
    with_speed(&mut f_obj);
    let p_obj = jsast::stmt_to_string(&f_obj);
    assert!(!p_obj.contains("bstr_out"), "{p_obj}");
    assert!(p_obj.contains("return o.s;"), "{p_obj}");

    // Negative 1: non-ASCII concat keeps bstr_out.
    let mut f_non_ascii = Stmt::FunctionDecl {
        name: "non_ascii".to_owned(),
        params: Vec::new(),
        body: vec![
            Stmt::Let("s".to_owned(), Some(jsast::string("a"))),
            jsast::expr_stmt(jsast::compound_assign(
                jsast::CompoundOp::Add,
                jsast::id("s"),
                jsast::string("\u{00c3}\u{00a9}"),
            )),
            jsast::ret(rt("bstr_out", vec![jsast::id("s")])),
        ],
    };
    with_speed(&mut f_non_ascii);
    assert!(
        jsast::stmt_to_string(&f_non_ascii).contains("bstr_out"),
        "{f_non_ascii}"
    );

    // Negative 2: unbounded String.fromCharCode(x) keeps bstr_out.
    let mut f_dyn = Stmt::FunctionDecl {
        name: "dyn_cc".to_owned(),
        params: vec!["x".to_owned()],
        body: vec![
            Stmt::Let(
                "s".to_owned(),
                Some(jsast::method_call(
                    jsast::id("String"),
                    "fromCharCode",
                    vec![jsast::id("x")],
                )),
            ),
            jsast::ret(rt("bstr_out", vec![jsast::id("s")])),
        ],
    };
    with_speed(&mut f_dyn);
    assert!(
        jsast::stmt_to_string(&f_dyn).contains("bstr_out"),
        "{f_dyn}"
    );

    // Negative 3: escaped native `{ s, cap }` object keeps bstr_out.
    let mut f_esc = Stmt::FunctionDecl {
        name: "esc_obj".to_owned(),
        params: vec!["ext".to_owned()],
        body: vec![
            Stmt::Let(
                "o".to_owned(),
                Some(jsast::object_of(vec![
                    ("s", jsast::string("abc")),
                    ("cap", jsast::num(3)),
                ])),
            ),
            jsast::expr_stmt(jsast::call(jsast::id("ext"), vec![jsast::id("o")])),
            jsast::ret(rt("bstr_out", vec![jsast::member(jsast::id("o"), "s")])),
        ],
    };
    with_speed(&mut f_esc);
    assert!(
        jsast::stmt_to_string(&f_esc).contains("bstr_out"),
        "{f_esc}"
    );

    // Negative 4: ungrounded self-cycle keeps bstr_out.
    let mut f_cyc = Stmt::FunctionDecl {
        name: "cyc".to_owned(),
        params: Vec::new(),
        body: vec![
            Stmt::Let("a".to_owned(), None),
            Stmt::Let("b".to_owned(), None),
            jsast::assign_stmt(jsast::id("a"), jsast::id("b")),
            jsast::assign_stmt(jsast::id("b"), jsast::id("a")),
            jsast::ret(rt("bstr_out", vec![jsast::id("a")])),
        ],
    };
    with_speed(&mut f_cyc);
    assert!(
        jsast::stmt_to_string(&f_cyc).contains("bstr_out"),
        "{f_cyc}"
    );
}

#[test]
fn elide_proven_bstr_sub_folds_guarded_and_loop_invariant_slices_soundly() {
    let with_speed = run_speed_passes;

    // Positive 1: dominating `s.charCodeAt(0) === 35` proves `bstr_sub_from(s, 1)` -> `s.slice(1)`.
    let mut f_prefix = Stmt::FunctionDecl {
        name: "strip_hash".to_owned(),
        params: vec!["s".to_owned()],
        body: vec![
            jsast::if_(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::num(0)]),
                    jsast::num(35),
                ),
                vec![jsast::ret(rt(
                    "bstr_sub_from",
                    vec![
                        jsast::id("s"),
                        jsast::num(1),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                ))],
            ),
            jsast::ret(jsast::id("s")),
        ],
    };
    with_speed(&mut f_prefix);
    let p_prefix = jsast::stmt_to_string(&f_prefix);
    assert!(p_prefix.contains("return s.slice(1);"), "{p_prefix}");
    assert!(!p_prefix.contains("bstr_sub"), "{p_prefix}");

    // Positive 2: `marked` `expand_tabs` loop-invariant `d` + `{ start, end }` range object.
    let mut f_tabs = Stmt::FunctionDecl {
        name: "expand_tabs".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            Stmt::Let("d".to_owned(), Some(jsast::num(0))),
            Stmt::Let("c".to_owned(), Some(jsast::num(0))),
            Stmt::Let("out".to_owned(), Some(jsast::string(""))),
            Stmt::LoopForever(
                None,
                vec![jsast::if_else(
                    jsast::binary(
                        jsast::BinOp::Lt,
                        jsast::id("c"),
                        jsast::member(jsast::id("a"), "length"),
                    ),
                    vec![
                        jsast::if_(
                            jsast::binary(
                                jsast::BinOp::StrictEq,
                                jsast::method_call(
                                    jsast::id("a"),
                                    "charCodeAt",
                                    vec![jsast::id("c")],
                                ),
                                jsast::num(9),
                            ),
                            vec![
                                jsast::if_(
                                    jsast::binary(jsast::BinOp::Gt, jsast::id("c"), jsast::id("d")),
                                    vec![
                                        Stmt::Let(
                                            "w".to_owned(),
                                            Some(jsast::object_of(vec![
                                                ("start", jsast::id("d")),
                                                ("end", jsast::id("c")),
                                            ])),
                                        ),
                                        jsast::expr_stmt(jsast::compound_assign(
                                            jsast::CompoundOp::Add,
                                            jsast::id("out"),
                                            rt(
                                                "bstr_sub",
                                                vec![
                                                    jsast::id("a"),
                                                    jsast::member(jsast::id("w"), "start"),
                                                    jsast::member(jsast::id("w"), "end"),
                                                    jsast::id("fail"),
                                                    jsast::id("loc"),
                                                ],
                                            ),
                                        )),
                                    ],
                                ),
                                jsast::assign_stmt(
                                    jsast::id("d"),
                                    jsast::binary(
                                        jsast::BinOp::UShr,
                                        jsast::binary(
                                            jsast::BinOp::Add,
                                            jsast::id("c"),
                                            jsast::num(1),
                                        ),
                                        jsast::num(0),
                                    ),
                                ),
                            ],
                        ),
                        jsast::assign_stmt(
                            jsast::id("c"),
                            jsast::binary(
                                jsast::BinOp::UShr,
                                jsast::binary(jsast::BinOp::Add, jsast::id("c"), jsast::num(1)),
                                jsast::num(0),
                            ),
                        ),
                    ],
                    vec![Stmt::Break(None)],
                )],
            ),
            jsast::ret(rt(
                "bstr_sub_from",
                vec![
                    jsast::id("a"),
                    jsast::id("d"),
                    jsast::id("fail"),
                    jsast::id("loc"),
                ],
            )),
        ],
    };
    with_speed(&mut f_tabs);
    let p_tabs = jsast::stmt_to_string(&f_tabs);
    assert!(!p_tabs.contains("bstr_sub"), "{p_tabs}");
    assert!(p_tabs.contains("a.slice(w.start, w.end)"), "{p_tabs}");
    assert!(p_tabs.contains("a.slice(d)"), "{p_tabs}");

    // Positive 3: `rtrim_byte` pattern (`c = a.length`, `c > 0 && a.charCodeAt((c - 1) >>> 0) === 32`).
    let mut f_rtrim = Stmt::FunctionDecl {
        name: "rtrim_byte".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            Stmt::Let(
                "c".to_owned(),
                Some(jsast::member(jsast::id("a"), "length")),
            ),
            Stmt::LoopForever(
                None,
                vec![jsast::if_else(
                    jsast::binary(
                        jsast::BinOp::And,
                        jsast::binary(jsast::BinOp::Gt, jsast::id("c"), jsast::num(0)),
                        jsast::binary(
                            jsast::BinOp::StrictEq,
                            jsast::method_call(
                                jsast::id("a"),
                                "charCodeAt",
                                vec![jsast::binary(
                                    jsast::BinOp::UShr,
                                    jsast::binary(jsast::BinOp::Sub, jsast::id("c"), jsast::num(1)),
                                    jsast::num(0),
                                )],
                            ),
                            jsast::num(32),
                        ),
                    ),
                    vec![jsast::assign_stmt(
                        jsast::id("c"),
                        jsast::binary(
                            jsast::BinOp::UShr,
                            jsast::binary(jsast::BinOp::Sub, jsast::id("c"), jsast::num(1)),
                            jsast::num(0),
                        ),
                    )],
                    vec![Stmt::Break(None)],
                )],
            ),
            jsast::ret(rt(
                "bstr_sub_to",
                vec![
                    jsast::id("a"),
                    jsast::id("c"),
                    jsast::id("fail"),
                    jsast::id("loc"),
                ],
            )),
        ],
    };
    with_speed(&mut f_rtrim);
    let p_rtrim = jsast::stmt_to_string(&f_rtrim);
    assert!(!p_rtrim.contains("bstr_sub"), "{p_rtrim}");
    assert!(p_rtrim.contains("a.slice(0, c)"), "{p_rtrim}");

    // Positive 4: trivial `0`/`s.length` and KnownAscii + bounds guard.
    let mut f_triv = Stmt::FunctionDecl {
        name: "triv".to_owned(),
        params: vec!["x".to_owned(), "i".to_owned()],
        body: vec![
            Stmt::Let("s".to_owned(), Some(jsast::string("hello"))),
            jsast::if_(
                jsast::binary(
                    jsast::BinOp::And,
                    jsast::binary(jsast::BinOp::Ge, jsast::id("i"), jsast::num(0)),
                    jsast::binary(
                        jsast::BinOp::Le,
                        jsast::id("i"),
                        jsast::member(jsast::id("s"), "length"),
                    ),
                ),
                vec![jsast::ret(rt(
                    "bstr_sub_from",
                    vec![
                        jsast::id("s"),
                        jsast::id("i"),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                ))],
            ),
            jsast::ret(jsast::array(vec![
                rt(
                    "bstr_sub_from",
                    vec![
                        jsast::id("x"),
                        jsast::num(0),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                ),
                rt(
                    "bstr_sub",
                    vec![
                        jsast::id("x"),
                        jsast::num(0),
                        jsast::member(jsast::id("x"), "length"),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                ),
                rt(
                    "bstr_sub_to",
                    vec![
                        jsast::id("x"),
                        jsast::num(0),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                ),
            ])),
        ],
    };
    with_speed(&mut f_triv);
    let p_triv = jsast::stmt_to_string(&f_triv);
    assert!(!p_triv.contains("bstr_sub"), "{p_triv}");
    assert!(p_triv.contains("return s.slice(i);"), "{p_triv}");
    assert!(p_triv.contains("[x, x, \"\"]"), "{p_triv}");

    // Negative 1: non-ASCII byte check (128) must NOT fold `bstr_sub_from(s, 1)`.
    let mut f_bad_byte = Stmt::FunctionDecl {
        name: "bad_byte".to_owned(),
        params: vec!["s".to_owned()],
        body: vec![
            jsast::if_(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::num(0)]),
                    jsast::num(128),
                ),
                vec![jsast::ret(rt(
                    "bstr_sub_from",
                    vec![
                        jsast::id("s"),
                        jsast::num(1),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                ))],
            ),
            jsast::ret(jsast::id("s")),
        ],
    };
    with_speed(&mut f_bad_byte);
    assert!(
        jsast::stmt_to_string(&f_bad_byte).contains("bstr_sub_from"),
        "{f_bad_byte}"
    );

    // Negative 2: KnownAscii without proven `a <= b <= s.length` must NOT fold `bstr_sub`.
    let mut f_bad_bounds = Stmt::FunctionDecl {
        name: "bad_bounds".to_owned(),
        params: vec!["a".to_owned(), "b".to_owned()],
        body: vec![
            Stmt::Let("s".to_owned(), Some(jsast::string("hello"))),
            jsast::ret(rt(
                "bstr_sub",
                vec![
                    jsast::id("s"),
                    jsast::id("a"),
                    jsast::id("b"),
                    jsast::id("fail"),
                    jsast::id("loc"),
                ],
            )),
        ],
    };
    with_speed(&mut f_bad_bounds);
    assert!(
        jsast::stmt_to_string(&f_bad_bounds).contains("bstr_sub"),
        "{f_bad_bounds}"
    );

    // Negative 3: u32 wraparound: checking `s.charCodeAt((b + 1) >>> 0) === 93` without
    // proving `b < s.length` must NOT prove `b` is a valid boundary (b could be 0xFFFF_FFFF).
    let mut f_wrap = Stmt::FunctionDecl {
        name: "bad_wrap".to_owned(),
        params: vec!["s".to_owned(), "b".to_owned()],
        body: vec![
            jsast::if_(
                jsast::binary(
                    jsast::BinOp::StrictEq,
                    jsast::method_call(
                        jsast::id("s"),
                        "charCodeAt",
                        vec![jsast::binary(
                            jsast::BinOp::UShr,
                            jsast::binary(jsast::BinOp::Add, jsast::id("b"), jsast::num(1)),
                            jsast::num(0),
                        )],
                    ),
                    jsast::num(93),
                ),
                vec![jsast::ret(rt(
                    "bstr_sub_from",
                    vec![
                        jsast::id("s"),
                        jsast::id("b"),
                        jsast::id("fail"),
                        jsast::id("loc"),
                    ],
                ))],
            ),
            jsast::ret(jsast::id("s")),
        ],
    };
    with_speed(&mut f_wrap);
    assert!(
        jsast::stmt_to_string(&f_wrap).contains("bstr_sub_from"),
        "{f_wrap}"
    );
}
