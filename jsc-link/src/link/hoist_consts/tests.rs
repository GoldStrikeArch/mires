#![allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-item link fixtures inline"
)]

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{item_map, ItemKind, JsName, LinkItem, Linkage};
use crate::jir::semantic::{CallSemantic, Callee, HostPayload, Terminator};
use crate::jir::types::{FieldKey, NominalForm, TypeId, TypeKind};
use crate::jir::{DefinitionId, Interns, ItemId};
use crate::jsast::{self, ArrowBody, BinOp, Expr, Stmt, SwitchCase, UnOp};
use crate::lower::LoweredItem;
use crate::opt::OptStats;
use crate::opts::{EcmaTarget, OptimizerMode, Opts};

use super::exclusions::{collect_js_text_tokens_and_strings, collect_program_exclusions};
use super::{run_with_opts, LoweredRow};

type TestRunOutput = (
    BTreeMap<JsName, LinkItem>,
    Vec<LoweredRow>,
    BTreeSet<JsName>,
    BTreeSet<JsName>,
    bool,
);

fn test_run(opts: Opts, items: Vec<(&str, Stmt)>) -> TestRunOutput {
    let mut table = BTreeMap::new();
    let mut defined = BTreeSet::new();
    let mut live = BTreeSet::new();
    let mut lowered = Vec::new();
    for (name, decl) in items {
        let js_name = JsName::new(name.to_owned());
        let refs = super::super::lane::refs_of(&decl, &js_name);
        defined.insert(js_name.clone());
        live.insert(js_name.clone());
        lowered.push((
            js_name,
            Ok(LoweredItem {
                decl,
                refs,
                zombies: Vec::new(),
                loc: None,
                names: Vec::new(),
                dts: None,
                js_alloc_origins: Vec::new(),
                bigints: BTreeSet::new(),
                hoisted_consts: BTreeMap::new(),
            }),
            OptStats::default(),
            None,
        ));
    }
    let added = run_with_opts(&mut table, &mut lowered, &mut defined, &mut live, &opts);
    (table, lowered, defined, live, added)
}

#[test]
fn hoists_nested_loop_lookup_table_to_module_const() {
    // `function crc(i) { while (true) { let t = [0,1,2,3,4,5,6,7]; return t[i]; } }`
    let table_elems: Vec<Expr> = (0..8).map(|n| Expr::Num(n as f64)).collect();
    let fn_stmt = jsast::function(
        "crc",
        vec!["i".to_owned()],
        vec![Stmt::LoopForever(
            None,
            vec![
                Stmt::Let("t".to_owned(), Some(Expr::Array(table_elems))),
                Stmt::Return(Some(Expr::Index(
                    Box::new(Expr::Ident("t".to_owned())),
                    Box::new(Expr::Ident("i".to_owned())),
                ))),
            ],
        )],
    );
    let opts = Opts {
        optimizer: OptimizerMode::On,
        ..Opts::default()
    };
    let (table, lowered, defined, live, added) = test_run(opts, vec![("crc", fn_stmt)]);
    assert!(added);
    assert_eq!(table.len(), 1);
    let (hc_name, hc_item) = table.iter().next().unwrap();
    assert_eq!(hc_item.kind, ItemKind::Const);
    assert!(
        hc_item.js.contains("[0, 1, 2, 3, 4, 5, 6, 7]") || hc_item.js.contains("[0,1,2,3,4,5,6,7]")
    );
    assert!(defined.contains(hc_name) && live.contains(hc_name));
    let Ok(item) = &lowered[0].1 else {
        unreachable!()
    };
    assert!(item.refs.contains(hc_name));
    let rendered = jsast::program_to_string_styled_for_target(
        std::slice::from_ref(&item.decl),
        jsast::Style::Compact,
        EcmaTarget::Latest,
    );
    assert!(
        !rendered.contains("let t"),
        "hoisted local declaration stripped: {rendered}"
    );
    assert!(rendered.contains(hc_name.as_str()));
}

#[test]
fn hoists_in_loop_location_bigint_es2018_and_shared_unreachable_helper() {
    let loc = Expr::Object(vec![
        ("filename".to_owned(), Expr::Str("lib.rs".to_owned())),
        ("line".to_owned(), Expr::Num(42.0)),
        ("col".to_owned(), Expr::Num(7.0)),
    ]);
    let unreachable_throw = || {
        Stmt::Throw(Expr::Call(
            Box::new(Expr::Member(
                Box::new(Expr::Ident("__rt".to_owned())),
                "abort_error".to_owned(),
            )),
            vec![Expr::New(
                Box::new(Expr::Ident("Error".to_owned())),
                vec![Expr::Str(
                    "rustc_codegen_js: entered unreachable code".to_owned(),
                )],
            )],
        ))
    };
    let fn_stmt = jsast::function(
        "demo",
        vec!["c".to_owned()],
        vec![Stmt::LoopForever(
            None,
            vec![
                Stmt::ExprStmt(Expr::Call(
                    Box::new(Expr::Ident("use_vals".to_owned())),
                    vec![loc, Expr::BigInt("12345678901234567890".to_owned())],
                )),
                Stmt::If(
                    Expr::Ident("c".to_owned()),
                    vec![unreachable_throw()],
                    Some(vec![unreachable_throw()]),
                ),
            ],
        )],
    );
    let opts = Opts {
        optimizer: OptimizerMode::On,
        ecma_target: EcmaTarget::Es2018,
        ..Opts::default()
    };
    let (table, lowered, _, _, added) = test_run(opts, vec![("demo", fn_stmt.clone())]);
    assert!(added);
    // Expect 1 constant (loc) and 1 unreachable helper function ($ur_0); BigInt constants are
    // left for `lower::bigint::hoist` so they get load-safe `$BI_ok`/`$BI_no()` guards.
    assert_eq!(table.len(), 2);
    assert_eq!(
        table.values().filter(|i| i.kind == ItemKind::Const).count(),
        1
    );
    assert_eq!(table.values().filter(|i| i.kind == ItemKind::Fn).count(), 1);
    let Ok(item) = &lowered[0].1 else {
        unreachable!()
    };
    let rendered = jsast::program_to_string_styled_for_target(
        std::slice::from_ref(&item.decl),
        jsast::Style::Compact,
        EcmaTarget::Es2018,
    );
    assert!(
        !rendered.contains("entered unreachable code"),
        "throws outlined: {rendered}"
    );
    assert!(
        rendered.contains("BigInt(\"12345678901234567890\")"),
        "BigInt left for lower::bigint::hoist on Es2018: {rendered}"
    );
    assert!(
        !rendered.contains("lib.rs"),
        "in-loop Location hoisted: {rendered}"
    );

    // Report mode must NOT hoist anything (preserves off == report byte-identity).
    let report_opts = Opts {
        optimizer: OptimizerMode::Report,
        ..Opts::default()
    };
    let (report_table, _, _, _, report_added) = test_run(report_opts, vec![("demo", fn_stmt)]);
    assert!(!report_added);
    assert!(report_table.is_empty());
}

#[test]
fn hoists_capture_free_zero_factory_arrows_to_module_const() {
    let zero_factory = Expr::Arrow(
        Vec::new(),
        ArrowBody::Expr(Box::new(Expr::Object(vec![
            ("_0".to_owned(), Expr::Str(String::new())),
            ("_1".to_owned(), Expr::Num(0.0)),
        ]))),
    );
    let fn_stmt = jsast::function(
        "push",
        vec!["p".to_owned()],
        vec![Stmt::Return(Some(Expr::Call(
            Box::new(Expr::Member(
                Box::new(Expr::Ident("__rt".to_owned())),
                "unscale".to_owned(),
            )),
            vec![Expr::Ident("p".to_owned()), Expr::Num(4.0), zero_factory],
        )))],
    );
    let opts = Opts {
        optimizer: OptimizerMode::On,
        ..Opts::default()
    };
    let (table, lowered, _, _, added) = test_run(opts, vec![("push", fn_stmt.clone())]);
    assert!(added);
    assert_eq!(table.len(), 1);
    let (hc_name, hc_item) = table.iter().next().unwrap();
    assert!(
        hc_item.js.contains("() => ({ _0: \"\", _1: 0 })"),
        "expected hoisted zero factory, got:\n{}",
        hc_item.js
    );
    let Ok(item) = &lowered[0].1 else {
        unreachable!()
    };
    let rendered = jsast::stmt_to_string(&item.decl);
    assert!(rendered.contains(hc_name.as_str()));
    assert!(!rendered.contains("=>"));

    let report_opts = Opts {
        optimizer: OptimizerMode::Report,
        ..Opts::default()
    };
    let (report_table, _, _, _, report_added) = test_run(report_opts, vec![("push", fn_stmt)]);
    assert!(!report_added);
    assert!(report_table.is_empty());
}

#[test]
fn splits_internal_bigint_struct_properties_including_materialized_temps() {
    let pack = |lo: Expr, hi: Expr| {
        Expr::Binary(
            BinOp::BitOr,
            Box::new(Expr::Binary(
                BinOp::Shl,
                Box::new(Expr::Call(
                    Box::new(Expr::Ident("BigInt".to_owned())),
                    vec![Expr::Binary(
                        BinOp::UShr,
                        Box::new(hi),
                        Box::new(Expr::Num(0.0)),
                    )],
                )),
                Box::new(Expr::BigInt("32".to_owned())),
            )),
            Box::new(Expr::Call(
                Box::new(Expr::Ident("BigInt".to_owned())),
                vec![Expr::Binary(
                    BinOp::UShr,
                    Box::new(lo),
                    Box::new(Expr::Num(0.0)),
                )],
            )),
        )
    };
    let unpack_lo = |src: Expr| {
        Expr::Binary(
            BinOp::BitOr,
            Box::new(Expr::Call(
                Box::new(Expr::Ident("Number".to_owned())),
                vec![Expr::Call(
                    Box::new(Expr::Member(
                        Box::new(Expr::Ident("BigInt".to_owned())),
                        "asIntN".to_owned(),
                    )),
                    vec![Expr::Num(32.0), src],
                )],
            )),
            Box::new(Expr::Num(0.0)),
        )
    };
    let unpack_hi = |src: Expr| {
        unpack_lo(Expr::Binary(
            BinOp::Shr,
            Box::new(src),
            Box::new(Expr::BigInt("32".to_owned())),
        ))
    };

    let make_fn = jsast::function(
        "make_searcher",
        vec!["lo".to_owned(), "hi".to_owned()],
        vec![Stmt::Return(Some(Expr::Object(vec![
            ("needle".to_owned(), Expr::Str("ab".to_owned())),
            (
                "byteset".to_owned(),
                pack(Expr::Ident("lo".to_owned()), Expr::Ident("hi".to_owned())),
            ),
            ("raw_prop".to_owned(), Expr::BigInt("7".to_owned())),
            ("escaped".to_owned(), Expr::BigInt("9".to_owned())),
        ])))],
    );
    let read_fn = jsast::function(
        "read_searcher",
        vec!["s".to_owned()],
        vec![
            Stmt::Lets(vec![("$t0".to_owned(), None)]),
            Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Ident("$t0".to_owned())),
                Box::new(Expr::Member(
                    Box::new(Expr::Ident("s".to_owned())),
                    "byteset".to_owned(),
                )),
            )),
            Stmt::Return(Some(Expr::Binary(
                BinOp::BitXor,
                Box::new(unpack_lo(Expr::Ident("$t0".to_owned()))),
                Box::new(unpack_hi(Expr::Ident("$t0".to_owned()))),
            ))),
        ],
    );
    let escape_fn = jsast::function(
        "escape_other",
        vec!["s".to_owned()],
        vec![Stmt::Return(Some(Expr::Member(
            Box::new(Expr::Ident("s".to_owned())),
            "escaped".to_owned(),
        )))],
    );

    let mut table = BTreeMap::new();
    let mut defined = BTreeSet::new();
    let mut live = BTreeSet::new();
    let mut lowered = Vec::new();
    for (name, decl) in [
        ("make_searcher", make_fn),
        ("read_searcher", read_fn),
        ("escape_other", escape_fn),
    ] {
        let js_name = JsName::new(name.to_owned());
        let refs = super::super::lane::refs_of(&decl, &js_name);
        defined.insert(js_name.clone());
        live.insert(js_name.clone());
        lowered.push((
            js_name,
            Ok(LoweredItem {
                decl,
                refs,
                zombies: Vec::new(),
                loc: None,
                names: Vec::new(),
                dts: None,
                js_alloc_origins: Vec::new(),
                bigints: BTreeSet::new(),
                hoisted_consts: BTreeMap::new(),
            }),
            OptStats::default(),
            None,
        ));
    }
    let raw_name = JsName::new("raw_reader".to_owned());
    defined.insert(raw_name.clone());
    live.insert(raw_name.clone());
    table.insert(
        raw_name.clone(),
        LinkItem {
            name: raw_name,
            kind: ItemKind::Fn,
            js: "function raw_reader(x) { return x.raw_prop; }".to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: String::new(),
            crate_name: String::new(),
            order: None,
            map: item_map(Vec::new()),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        },
    );

    let opts = Opts {
        optimizer: OptimizerMode::On,
        ecma_target: EcmaTarget::Es2018,
        ..Opts::default()
    };
    let _ = run_with_opts(&mut table, &mut lowered, &mut defined, &mut live, &opts);

    let make_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
    let read_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
    assert!(make_js.contains("byteset$lo: lo | 0"), "{make_js}");
    assert!(make_js.contains("byteset$hi: hi | 0"), "{make_js}");
    assert!(!make_js.contains("byteset:"), "{make_js}");
    assert!(make_js.contains("raw_prop:"), "{make_js}");
    assert!(make_js.contains("escaped:"), "{make_js}");
    assert!(read_js.contains("$t0$lo"), "{read_js}");
    assert!(read_js.contains("$t0$hi"), "{read_js}");
    assert!(!read_js.contains("BigInt"), "{read_js}");
    assert!(!read_js.contains(".byteset\n"), "{read_js}");
}

#[test]
fn encodes_internal_enum_tags_as_dense_ints_while_respecting_exclusions() {
    let fn_a = jsast::function(
        "use_internal",
        vec!["s".to_owned()],
        vec![
            Stmt::Let(
                "v".to_owned(),
                Some(Expr::Object(vec![(
                    "TAG".to_owned(),
                    Expr::Str("Alpha".to_owned()),
                )])),
            ),
            Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Member(
                    Box::new(Expr::Ident("v".to_owned())),
                    "TAG".to_owned(),
                )),
                Box::new(Expr::Str("Beta".to_owned())),
            )),
            Stmt::ExprStmt(Expr::Binary(
                BinOp::StrictEq,
                Box::new(Expr::Member(
                    Box::new(Expr::Ident("s".to_owned())),
                    "TAG".to_owned(),
                )),
                Box::new(Expr::Str("Some".to_owned())),
            )),
            Stmt::Switch(
                Expr::Member(Box::new(Expr::Ident("s".to_owned())), "TAG".to_owned()),
                vec![
                    SwitchCase {
                        tests: vec![Expr::Str("Alpha".to_owned())],
                        body: vec![Stmt::Return(Some(Expr::Num(10.0)))],
                    },
                    SwitchCase {
                        tests: vec![Expr::Str("Leaked".to_owned())],
                        body: vec![Stmt::Return(Some(Expr::Str("Leaked".to_owned())))],
                    },
                    SwitchCase {
                        tests: vec![Expr::Str("FromRaw".to_owned())],
                        body: vec![Stmt::Return(Some(Expr::Num(30.0)))],
                    },
                ],
            ),
        ],
    );

    let mut table = BTreeMap::new();
    let mut defined = BTreeSet::new();
    let mut live = BTreeSet::new();
    let js_name = JsName::new("use_internal".to_owned());
    let refs = super::super::lane::refs_of(&fn_a, &js_name);
    defined.insert(js_name.clone());
    live.insert(js_name.clone());
    let mut lowered = vec![(
        js_name,
        Ok(LoweredItem {
            decl: fn_a.clone(),
            refs,
            zombies: Vec::new(),
            loc: None,
            names: Vec::new(),
            dts: None,
            js_alloc_origins: Vec::new(),
            bigints: BTreeSet::new(),
            hoisted_consts: BTreeMap::new(),
        }),
        OptStats::default(),
        None,
    )];
    let raw_name = JsName::new("raw_fn".to_owned());
    defined.insert(raw_name.clone());
    live.insert(raw_name.clone());
    table.insert(
        raw_name.clone(),
        LinkItem {
            name: raw_name,
            kind: ItemKind::Fn,
            js: "function raw_fn(x) { return x.TAG === \"FromRaw\"; }".to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: String::new(),
            crate_name: String::new(),
            order: None,
            map: item_map(Vec::new()),
            dts: None,
            optimization: Default::default(),
            allocator_provider: Default::default(),
            jir: Default::default(),
        },
    );

    let opts = Opts {
        optimizer: OptimizerMode::On,
        ..Opts::default()
    };
    let _ = run_with_opts(&mut table, &mut lowered, &mut defined, &mut live, &opts);
    let rendered = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
    assert!(rendered.contains("{ TAG: 0 }"), "{rendered}");
    assert!(rendered.contains("v.TAG = 1;"), "{rendered}");
    assert!(rendered.contains("case 0:"), "{rendered}");
    assert!(rendered.contains("s.TAG === \"Some\""), "{rendered}");
    assert!(rendered.contains("case \"Leaked\":"), "{rendered}");
    assert!(rendered.contains("case \"FromRaw\":"), "{rendered}");

    let report_opts = Opts {
        optimizer: OptimizerMode::Report,
        ..Opts::default()
    };
    let (_, report_lowered, _, _, _) = test_run(report_opts, vec![("use_internal", fn_a.clone())]);
    let report_js = jsast::stmt_to_string(&report_lowered[0].1.as_ref().unwrap().decl);
    assert!(report_js.contains("\"Alpha\""), "{report_js}");
    assert!(report_js.contains("\"Beta\""), "{report_js}");

    // Under ReleaseSpeed, internal `"Some"`/`"None"` tags are integer-encoded too unless excluded.
    let speed_opts = Opts {
        optimizer: OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseSpeed),
        ..Opts::default()
    };
    let (_, speed_lowered, _, _, _) = test_run(speed_opts, vec![("use_internal", fn_a)]);
    let speed_js = jsast::stmt_to_string(&speed_lowered[0].1.as_ref().unwrap().decl);
    assert!(!speed_js.contains("\"Some\""), "{speed_js}");
    assert!(speed_js.contains("s.TAG ==="), "{speed_js}");
}

#[test]
fn collect_program_exclusions_protects_js_extern_host_call_and_closure_types() {
    use crate::item::LinkJir;
    use crate::jir::semantic::{
        AbiSignature, BasicBlock, BlockId, BodySemantic, Call, Constant, Local, LocalId, LocalRole,
        Operand, OperandKind, Place, Signature, Unwind,
    };
    use crate::jir::types::{
        EnumRepr, FieldDescriptor, IntFacts, IntRepr, JsFacts, LayoutFacts, NominalDiscriminant,
        TypeDescriptor, TypeMeta, VariantDescriptor,
    };
    use crate::jir::{Body, BodyKind, BytesId};

    let mut interns = Interns::default();
    let u64_ty = TypeId(1);
    let struct_ty = TypeId(2);
    let enum_ty = TypeId(3);
    let closure_ty = TypeId(4);
    let cb_struct_ty = TypeId(5);
    let cb_enum_ty = TypeId(6);
    let unit_ty = TypeId(7);
    let fn_ty = TypeId(8);

    let def_of = |n: u64| DefinitionId {
        crate_id: 1,
        local_hash: n,
    };
    let mk_desc = |id: TypeId, kind: TypeKind, size: u64| TypeDescriptor {
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
    };
    interns.intern_type(mk_desc(unit_ty, TypeKind::Tuple(Vec::new()), 0));
    let mut u64_desc = mk_desc(
        u64_ty,
        TypeKind::Integer {
            signed: false,
            bits: 64,
        },
        8,
    );
    u64_desc.js.int = Some(IntFacts {
        signed: false,
        bits: 64,
        repr: IntRepr::BigInt,
    });
    interns.intern_type(u64_desc);
    interns.intern_type(mk_desc(fn_ty, TypeKind::Opaque, 0));

    let ts_key = interns.intern_str("ext_ts");
    let cb_ts_key = interns.intern_str("cb_ts");
    let go_key = interns.intern_str("Go");
    let stop_key = interns.intern_str("Stop");
    let cb_done_key = interns.intern_str("CbDone");
    let extern_struct_key = interns.intern_str("ExternStruct");
    let cb_struct_key = interns.intern_str("CbStruct");

    interns.intern_type(mk_desc(
        struct_ty,
        TypeKind::Adt {
            def: def_of(10),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![VariantDescriptor {
                name: extern_struct_key,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(ts_key),
                    ty: u64_ty,
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
    ));
    let mut enum_desc = mk_desc(
        enum_ty,
        TypeKind::Adt {
            def: def_of(11),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![
                VariantDescriptor {
                    name: go_key,
                    fields: Vec::new(),
                    logical_discr: 0,
                    physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
                VariantDescriptor {
                    name: stop_key,
                    fields: Vec::new(),
                    logical_discr: 1,
                    physical_tag: Some(NominalDiscriminant { bits: 1, bytes: 1 }),
                    result_ok: false,
                    saved_local_keys: Vec::new(),
                },
            ],
            niche: None,
        },
        1,
    );
    enum_desc.js.enum_repr = Some(EnumRepr::Tag { fieldless: true });
    interns.intern_type(enum_desc);
    interns.intern_type(mk_desc(
        cb_struct_ty,
        TypeKind::Adt {
            def: def_of(12),
            args: Vec::new(),
            form: NominalForm::Struct,
            variants: vec![VariantDescriptor {
                name: cb_struct_key,
                fields: vec![FieldDescriptor {
                    key: FieldKey::Name(cb_ts_key),
                    ty: u64_ty,
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
    ));
    interns.intern_type(mk_desc(
        cb_enum_ty,
        TypeKind::Adt {
            def: def_of(13),
            args: Vec::new(),
            form: NominalForm::Enum,
            variants: vec![VariantDescriptor {
                name: cb_done_key,
                fields: Vec::new(),
                logical_discr: 0,
                physical_tag: Some(NominalDiscriminant { bits: 0, bytes: 1 }),
                result_ok: false,
                saved_local_keys: Vec::new(),
            }],
            niche: None,
        },
        1,
    ));

    let closure_def = def_of(20);
    let closure_item_id = ItemId {
        definition: closure_def,
        instantiation: closure_ty.0,
    };
    interns.intern_type(mk_desc(
        closure_ty,
        TypeKind::Closure {
            def: closure_def,
            upvars: Vec::new(),
            fn_pointer_shim: None,
        },
        8,
    ));

    let caller_item_id = ItemId {
        definition: def_of(30),
        instantiation: 1,
    };
    let host_section = interns.intern_str("rcgjs.host.callback0");
    let caller_body = Body {
        item: caller_item_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: Signature {
            arguments: Vec::new(),
            result: unit_ty,
            abi: AbiSignature::default(),
        },
        locals: vec![Local {
            id: LocalId(0),
            role: LocalRole::Return,
            ty: enum_ty,
        }],
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: fn_ty,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    callee: Callee::Indirect { signature: fn_ty },
                    semantic: CallSemantic::Host,
                    arguments: vec![Operand {
                        ty: struct_ty,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    }],
                    destination: Place {
                        local: LocalId(0),
                        projection: Vec::new(),
                        ty: enum_ty,
                    },
                    target: Some(BlockId(1)),
                    unwind: Unwind::Continue,
                    track_caller: None,
                    host: Some(HostPayload::Extern(BytesId(0))),
                }),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: fn_ty,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    callee: Callee::Indirect { signature: fn_ty },
                    semantic: CallSemantic::Host,
                    arguments: vec![Operand {
                        ty: closure_ty,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    }],
                    destination: Place {
                        local: LocalId(0),
                        projection: Vec::new(),
                        ty: unit_ty,
                    },
                    target: Some(BlockId(2)),
                    unwind: Unwind::Continue,
                    track_caller: None,
                    host: Some(HostPayload::Marker {
                        section: host_section,
                        def: def_of(40),
                        helpers: Vec::new(),
                    }),
                }),
            },
            BasicBlock {
                id: BlockId(2),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            },
        ],
    };

    let closure_body = Body {
        item: closure_item_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: Signature {
            arguments: vec![cb_struct_ty],
            result: cb_enum_ty,
            abi: AbiSignature::default(),
        },
        locals: Vec::new(),
        blocks: Vec::new(),
    };

    let caller_name = JsName::new("internal_caller".to_owned());
    let closure_name = JsName::new("cb_closure".to_owned());
    let mut table = BTreeMap::new();
    for (name, body) in [
        (caller_name.clone(), caller_body),
        (closure_name.clone(), closure_body),
    ] {
        let mut jir = LinkJir::default();
        jir.use_interns(interns.clone());
        jir.body = Some(body);
        table.insert(
            name.clone(),
            LinkItem {
                name,
                kind: ItemKind::Fn,
                js: String::new(),
                refs: BTreeSet::new(),
                zombies: Vec::new(),
                linkage: Linkage::internal(),
                debug_path: String::new(),
                crate_name: String::new(),
                order: None,
                map: item_map(Vec::new()),
                dts: None,
                optimization: Default::default(),
                allocator_provider: Default::default(),
                jir,
            },
        );
    }
    let live: BTreeSet<JsName> = [caller_name.clone(), closure_name].into_iter().collect();
    let lowered = vec![(
        caller_name,
        Ok(LoweredItem {
            decl: Stmt::Comment(String::new()),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            loc: None,
            names: Vec::new(),
            dts: None,
            js_alloc_origins: Vec::new(),
            bigints: BTreeSet::new(),
            hoisted_consts: BTreeMap::new(),
        }),
        OptStats::default(),
        None,
    )];

    let exclusions = collect_program_exclusions(&table, &lowered, &live);
    assert!(exclusions.boundary_fields.contains("ext_ts"));
    assert!(exclusions.boundary_fields.contains("cb_ts"));
    assert!(exclusions.boundary_variants.contains("Go"));
    assert!(exclusions.boundary_variants.contains("Stop"));
    assert!(exclusions.boundary_variants.contains("CbDone"));
}

#[test]
fn collect_js_text_tokens_and_strings_handles_comments_templates_escapes_and_lo_hi() {
    let mut tokens = BTreeSet::new();
    let mut strings = BTreeSet::new();
    let text = r#"
        // caller's frame shouldn't open a fake single-quoted string
        /* another's 'block' comment */
        const tpl = `TplOnly`;
        const interp = `head ${fn(inner_tok, "QuotedInTpl")}`;
        const s1 = "\u0046oo";
        const s2 = "a\"b";
        const x = { foo$lo: 1, after_comment: 2 };
    "#;
    collect_js_text_tokens_and_strings(text, &mut tokens, &mut strings);
    assert!(tokens.contains("after_comment"));
    assert!(tokens.contains("inner_tok"));
    assert!(tokens.contains("foo$lo"));
    assert!(strings.contains("Foo"));
    assert!(strings.contains("a\"b"));
    assert!(strings.contains("TplOnly"));
    assert!(strings.contains("QuotedInTpl"));

    // Ensure `foo$lo` exclusion also blocks splitting `foo`.
    let make_fn = jsast::function(
        "mk",
        vec![],
        vec![Stmt::Return(Some(Expr::Object(vec![
            ("foo".to_owned(), Expr::BigInt("42".to_owned())),
            ("bar".to_owned(), Expr::BigInt("42".to_owned())),
        ])))],
    );
    let (_, lowered, _, _, _) = test_run(
        Opts {
            optimizer: OptimizerMode::On,
            ecma_target: EcmaTarget::Es2018,
            ..Opts::default()
        },
        vec![
            ("mk", make_fn),
            (
                "guard",
                jsast::function(
                    "guard",
                    vec![],
                    vec![Stmt::Return(Some(Expr::Str("foo$lo".to_owned())))],
                ),
            ),
        ],
    );
    let mk_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
    assert!(
        mk_js.contains("foo:"),
        "foo$lo must block splitting foo:\n{mk_js}"
    );
    assert!(
        mk_js.contains("bar$lo:"),
        "unrelated bar should still split:\n{mk_js}"
    );
}

#[test]
fn bigint_split_ignores_captured_arrow_locals_and_signed_lo_packs() {
    let unpack_lo = |src: Expr| {
        Expr::Binary(
            BinOp::BitOr,
            Box::new(Expr::Call(
                Box::new(Expr::Ident("Number".to_owned())),
                vec![Expr::Call(
                    Box::new(Expr::Member(
                        Box::new(Expr::Ident("BigInt".to_owned())),
                        "asIntN".to_owned(),
                    )),
                    vec![Expr::Num(32.0), src],
                )],
            )),
            Box::new(Expr::Num(0.0)),
        )
    };
    let signed_pack = Expr::Binary(
        BinOp::BitOr,
        Box::new(Expr::Binary(
            BinOp::Shl,
            Box::new(Expr::Call(
                Box::new(Expr::Ident("BigInt".to_owned())),
                vec![Expr::Ident("hi".to_owned())],
            )),
            Box::new(Expr::BigInt("32".to_owned())),
        )),
        Box::new(Expr::Call(
            Box::new(Expr::Ident("BigInt".to_owned())),
            vec![Expr::Num(-1.0)],
        )),
    );
    let valid_pack = Expr::Binary(
        BinOp::BitOr,
        Box::new(Expr::Binary(
            BinOp::Shl,
            Box::new(Expr::Call(
                Box::new(Expr::Ident("BigInt".to_owned())),
                vec![Expr::Ident("hi".to_owned())],
            )),
            Box::new(Expr::BigInt("32".to_owned())),
        )),
        Box::new(Expr::Call(
            Box::new(Expr::Ident("BigInt".to_owned())),
            vec![Expr::Binary(
                BinOp::UShr,
                Box::new(Expr::Ident("lo".to_owned())),
                Box::new(Expr::Num(0.0)),
            )],
        )),
    );

    let fn_maker = jsast::function(
        "mk",
        vec!["lo".to_owned(), "hi".to_owned()],
        vec![Stmt::Return(Some(Expr::Object(vec![
            ("signed_bad".to_owned(), signed_pack),
            ("split_ok".to_owned(), valid_pack),
            ("byteset".to_owned(), Expr::BigInt("1".to_owned())),
        ])))],
    );
    let fn_capture = jsast::function(
        "capture_temp",
        vec!["obj".to_owned()],
        vec![
            Stmt::Lets(vec![("$t0".to_owned(), None)]),
            Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Ident("$t0".to_owned())),
                Box::new(Expr::Member(
                    Box::new(Expr::Ident("obj".to_owned())),
                    "byteset".to_owned(),
                )),
            )),
            Stmt::Return(Some(Expr::Arrow(
                Vec::new(),
                ArrowBody::Expr(Box::new(unpack_lo(Expr::Ident("$t0".to_owned())))),
            ))),
        ],
    );

    let (_, lowered, _, _, _) = test_run(
        Opts {
            optimizer: OptimizerMode::On,
            ecma_target: EcmaTarget::Es2018,
            ..Opts::default()
        },
        vec![("mk", fn_maker), ("capture_temp", fn_capture)],
    );
    let mk_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
    let cap_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
    assert!(
        mk_js.contains("signed_bad:"),
        "negative lo_arg must NOT split:\n{mk_js}"
    );
    assert!(
        !mk_js.contains("signed_bad$lo"),
        "negative lo_arg must NOT emit signed_bad$lo:\n{mk_js}"
    );
    assert!(
        mk_js.contains("split_ok$lo:") && mk_js.contains("split_ok$hi:"),
        "unsigned lo_arg pack should still split:\n{mk_js}"
    );
    assert!(
        mk_js.contains("byteset:") && !mk_js.contains("byteset$lo"),
        "captured temp reading obj.byteset must prevent splitting byteset:\n{mk_js}"
    );
    assert!(
        cap_js.contains("let $t0;") && !cap_js.contains("$t0$lo"),
        "captured temp inside arrow must stay declared as $t0:\n{cap_js}"
    );
}

#[test]
fn read_only_nested_const_slice_hoists_and_mutated_slice_does_not() {
    let make_slice = || {
        Expr::Object(vec![
            (
                "$b".to_owned(),
                Expr::Array(
                    (0..4)
                        .map(|i| Expr::Array(vec![Expr::Num(i as f64), Expr::Num((i * 10) as f64)]))
                        .collect(),
                ),
            ),
            ("$o".to_owned(), Expr::Num(0.0)),
            ("$l".to_owned(), Expr::Num(4.0)),
        ])
    };

    // Positive: `s = { $b: [[0,0], ...], $o: 0, $l: 4 }; return s.$b[s.$o + i][0];`
    let fn_ro = jsast::function(
        "read_iota",
        vec!["i".to_owned()],
        vec![
            Stmt::Lets(vec![("s".to_owned(), None)]),
            Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Ident("s".to_owned())),
                Box::new(make_slice()),
            )),
            Stmt::Return(Some(Expr::Index(
                Box::new(Expr::Index(
                    Box::new(Expr::Member(
                        Box::new(Expr::Ident("s".to_owned())),
                        "$b".to_owned(),
                    )),
                    Box::new(Expr::Binary(
                        BinOp::Add,
                        Box::new(Expr::Member(
                            Box::new(Expr::Ident("s".to_owned())),
                            "$o".to_owned(),
                        )),
                        Box::new(Expr::Ident("i".to_owned())),
                    )),
                )),
                Box::new(Expr::Num(0.0)),
            ))),
        ],
    );

    // Negative: mutates `s.$b[0][0] = 99` -> MUST NOT hoist!
    let fn_mut = jsast::function(
        "write_iota",
        vec![],
        vec![
            Stmt::Lets(vec![("s".to_owned(), None)]),
            Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Ident("s".to_owned())),
                Box::new(make_slice()),
            )),
            Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Index(
                    Box::new(Expr::Index(
                        Box::new(Expr::Member(
                            Box::new(Expr::Ident("s".to_owned())),
                            "$b".to_owned(),
                        )),
                        Box::new(Expr::Num(0.0)),
                    )),
                    Box::new(Expr::Num(0.0)),
                )),
                Box::new(Expr::Num(99.0)),
            )),
            Stmt::Return(Some(Expr::Member(
                Box::new(Expr::Ident("s".to_owned())),
                "$l".to_owned(),
            ))),
        ],
    );

    let (consts, lowered, _, _, _) = test_run(
        Opts {
            optimizer: OptimizerMode::On,
            ..Opts::default()
        },
        vec![("read_iota", fn_ro), ("write_iota", fn_mut)],
    );
    let ro_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
    let mut_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
    assert!(
        !consts.is_empty() && ro_js.contains("$hc_"),
        "read-only nested const slice should hoist:\nconsts={consts:?}\nro={ro_js}"
    );
    assert!(
        !mut_js.contains("$hc_") && mut_js.contains("$b:"),
        "mutated nested slice must NOT hoist:\nmut={mut_js}"
    );
}

#[test]
fn speed_profile_wraps_readonly_hoisted_and_static_int_tables_in_typed_arrays() {
    let crc_vals: Vec<Expr> = vec![
        0.0,
        1996959894.0,
        3993919788.0,
        2567524794.0,
        124634137.0,
        1886057615.0,
        3915621685.0,
        2657392035.0,
    ]
    .into_iter()
    .map(Expr::Num)
    .collect();
    let small_vals: Vec<Expr> = (0..8).map(|n| Expr::Num(f64::from(n))).collect();

    let static_ro = Stmt::Let("CRCT".to_owned(), Some(Expr::Array(crc_vals)));
    let static_mut = Stmt::Let("MUT_T".to_owned(), Some(Expr::Array(small_vals.clone())));
    let fn_use = jsast::function(
        "use_tables",
        vec!["i".to_owned()],
        vec![
            Stmt::Let("local_t".to_owned(), Some(Expr::Array(small_vals))),
            Stmt::ExprStmt(Expr::Assign(
                Box::new(Expr::Index(
                    Box::new(Expr::Ident("MUT_T".to_owned())),
                    Box::new(Expr::Ident("i".to_owned())),
                )),
                Box::new(Expr::Num(7.0)),
            )),
            Stmt::Return(Some(Expr::Binary(
                BinOp::BitXor,
                Box::new(Expr::Index(
                    Box::new(Expr::Ident("CRCT".to_owned())),
                    Box::new(Expr::Ident("i".to_owned())),
                )),
                Box::new(Expr::Index(
                    Box::new(Expr::Ident("local_t".to_owned())),
                    Box::new(Expr::Ident("i".to_owned())),
                )),
            ))),
        ],
    );

    let speed_opts = Opts {
        optimizer: OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseSpeed),
        ..Opts::default()
    };
    let (table, lowered, _, _, _) = test_run(
        speed_opts,
        vec![
            ("CRCT", static_ro),
            ("MUT_T", static_mut),
            ("use_tables", fn_use),
        ],
    );

    let hc_item = table
        .values()
        .find(|it| it.name.as_str().starts_with("$hc_"))
        .expect("local_t hoisted");
    assert!(
        hc_item.js.contains("new Uint8Array("),
        "expected local small-int table wrapped in Uint8Array under ReleaseSpeed: {}",
        hc_item.js
    );

    let crct_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
    assert!(
        crct_js.starts_with("const CRCT = new Uint32Array("),
        "expected read-only static CRCT upgraded to const Uint32Array: {crct_js}"
    );

    let mut_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
    assert!(
        mut_js.starts_with("let MUT_T = ["),
        "mutated static array must stay plain let Array: {mut_js}"
    );
}

#[test]
fn speed_profile_hoists_small_loop_const_arrays_and_skips_mutable_or_escaping() {
    // Exact `minisearch::text::tokenize` 4-word ASCII bitmask allocated inside a loop, plus
    // a 4-element Float64 lookup table, alongside a mutable `[0, 0, 0, 0]` and an escaping
    // `[1, 2, 3, 4]` that MUST NOT be hoisted.
    let mask_vals: Vec<Expr> = [9216.0, 2348873711.0, 3087007745.0, 671088640.0]
        .into_iter()
        .map(Expr::Num)
        .collect();
    let float_vals: Vec<Expr> = [
        Expr::Num(0.5),
        Expr::Num(1.5),
        Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2.5))),
        Expr::Num(3.5),
    ]
    .into_iter()
    .collect();
    let zero4: Vec<Expr> = vec![Expr::Num(0.0); 4];
    let esc4: Vec<Expr> = [1.0, 2.0, 3.0, 4.0].into_iter().map(Expr::Num).collect();

    let fn_tokenize = jsast::function(
        "tokenize",
        vec!["j".to_owned(), "n".to_owned()],
        vec![
            Stmt::Lets(vec![
                ("ar".to_owned(), None),
                ("fl".to_owned(), None),
                ("mut_a".to_owned(), None),
                ("esc_a".to_owned(), None),
                ("i".to_owned(), Some(Expr::Num(0.0))),
                ("acc".to_owned(), Some(Expr::Num(0.0))),
            ]),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(Expr::Ident("i".to_owned())),
                    Box::new(Expr::Ident("n".to_owned())),
                ),
                vec![
                    Stmt::ExprStmt(Expr::Assign(
                        Box::new(Expr::Ident("ar".to_owned())),
                        Box::new(Expr::Array(mask_vals)),
                    )),
                    Stmt::ExprStmt(Expr::Assign(
                        Box::new(Expr::Ident("fl".to_owned())),
                        Box::new(Expr::Array(float_vals)),
                    )),
                    Stmt::ExprStmt(Expr::Assign(
                        Box::new(Expr::Ident("mut_a".to_owned())),
                        Box::new(Expr::Array(zero4)),
                    )),
                    Stmt::ExprStmt(Expr::Assign(
                        Box::new(Expr::Index(
                            Box::new(Expr::Ident("mut_a".to_owned())),
                            Box::new(Expr::Num(0.0)),
                        )),
                        Box::new(Expr::Num(1.0)),
                    )),
                    Stmt::ExprStmt(Expr::Assign(
                        Box::new(Expr::Ident("esc_a".to_owned())),
                        Box::new(Expr::Array(esc4)),
                    )),
                    Stmt::ExprStmt(Expr::Call(
                        Box::new(Expr::Ident("sink".to_owned())),
                        vec![Expr::Ident("esc_a".to_owned())],
                    )),
                    Stmt::ExprStmt(Expr::Assign(
                        Box::new(Expr::Ident("acc".to_owned())),
                        Box::new(Expr::Binary(
                            BinOp::Add,
                            Box::new(Expr::Index(
                                Box::new(Expr::Ident("ar".to_owned())),
                                Box::new(Expr::Ident("j".to_owned())),
                            )),
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(Expr::Index(
                                    Box::new(Expr::Ident("fl".to_owned())),
                                    Box::new(Expr::Ident("j".to_owned())),
                                )),
                                Box::new(Expr::Index(
                                    Box::new(Expr::Ident("mut_a".to_owned())),
                                    Box::new(Expr::Ident("j".to_owned())),
                                )),
                            )),
                        )),
                    )),
                    Stmt::ExprStmt(Expr::Assign(
                        Box::new(Expr::Ident("i".to_owned())),
                        Box::new(Expr::Binary(
                            BinOp::Add,
                            Box::new(Expr::Ident("i".to_owned())),
                            Box::new(Expr::Num(1.0)),
                        )),
                    )),
                ],
            ),
            Stmt::Return(Some(Expr::Ident("acc".to_owned()))),
        ],
    );

    let speed_opts = Opts {
        optimizer: OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseSpeed),
        ..Opts::default()
    };
    let (table, lowered, _, _, added) = test_run(speed_opts, vec![("tokenize", fn_tokenize)]);
    assert!(added, "expected hoisted const items");
    let fn_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);

    let all_consts: String = table
        .values()
        .map(|it| it.js.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all_consts.contains("new Uint32Array([9216, 2348873711, 3087007745, 671088640])"),
        "expected 4-element u32 mask hoisted as Uint32Array:\n{all_consts}"
    );
    assert!(
        all_consts.contains("new Float64Array([0.5, 1.5, -2.5, 3.5])"),
        "expected 4-element float table hoisted as Float64Array:\n{all_consts}"
    );
    assert!(
        !fn_js.contains("9216") && !fn_js.contains("2348873711") && !fn_js.contains("-2.5"),
        "hoisted array literals must be stripped from loop body:\n{fn_js}"
    );
    assert!(
        fn_js.contains("mut_a = [0, 0, 0, 0];"),
        "mutated [0, 0, 0, 0] must NOT be hoisted:\n{fn_js}"
    );
    assert!(
        fn_js.contains("esc_a = [1, 2, 3, 4];"),
        "escaping [1, 2, 3, 4] passed to call must NOT be hoisted:\n{fn_js}"
    );
}

#[test]
fn hoists_and_deduplicates_inline_u8_luts_to_module_scope() {
    let mut lut_elems = vec![Expr::Num(0.0); 128];
    for &b in &[33usize, 42, 60, 91, 92, 95, 96, 126] {
        lut_elems[b] = Expr::Num(1.0);
    }
    let mk_lut = || {
        Expr::New(
            Box::new(Expr::Ident("Uint8Array".to_owned())),
            vec![Expr::Array(lut_elems.clone())],
        )
    };
    let fn_a = jsast::function(
        "scan_a",
        vec!["c".to_owned()],
        vec![Stmt::Return(Some(Expr::Binary(
            BinOp::StrictEq,
            Box::new(Expr::Index(
                Box::new(mk_lut()),
                Box::new(Expr::Ident("c".to_owned())),
            )),
            Box::new(Expr::Num(1.0)),
        )))],
    );
    let fn_b = jsast::function(
        "scan_b",
        vec!["c".to_owned()],
        vec![Stmt::Return(Some(Expr::Binary(
            BinOp::StrictEq,
            Box::new(Expr::Index(
                Box::new(mk_lut()),
                Box::new(Expr::Ident("c".to_owned())),
            )),
            Box::new(Expr::Num(1.0)),
        )))],
    );
    // Shadowed `Uint8Array` param MUST NOT be hoisted.
    let fn_shadowed = jsast::function(
        "scan_shadowed",
        vec!["Uint8Array".to_owned(), "c".to_owned()],
        vec![Stmt::Return(Some(Expr::Binary(
            BinOp::StrictEq,
            Box::new(Expr::Index(
                Box::new(mk_lut()),
                Box::new(Expr::Ident("c".to_owned())),
            )),
            Box::new(Expr::Num(1.0)),
        )))],
    );
    // Assignment target `new Uint8Array([...])[c] = 1` MUST NOT be hoisted.
    let fn_write = jsast::function(
        "write_lut",
        vec!["c".to_owned()],
        vec![Stmt::ExprStmt(Expr::Assign(
            Box::new(Expr::Index(
                Box::new(mk_lut()),
                Box::new(Expr::Ident("c".to_owned())),
            )),
            Box::new(Expr::Num(1.0)),
        ))],
    );

    let speed_opts = Opts {
        optimizer: OptimizerMode::On,
        profile: Some(crate::opts::Profile::ReleaseSpeed),
        ..Opts::default()
    };
    let (table, lowered, _, _, added) = test_run(
        speed_opts,
        vec![
            ("scan_a", fn_a),
            ("scan_b", fn_b),
            ("scan_shadowed", fn_shadowed),
            ("write_lut", fn_write),
        ],
    );
    assert!(added, "expected hoisted Uint8Array LUT item");
    let hc_items: Vec<&LinkItem> = table
        .values()
        .filter(|it| it.name.as_str().starts_with("$hc_"))
        .collect();
    assert_eq!(
        hc_items.len(),
        1,
        "expected identical 128-byte LUTs deduplicated into 1 module const: {:?}",
        hc_items.iter().map(|i| &i.js).collect::<Vec<_>>()
    );
    assert!(
        hc_items[0].js.contains("new Uint8Array("),
        "expected module const initialized with new Uint8Array: {}",
        hc_items[0].js
    );

    let a_js = jsast::stmt_to_string(&lowered[0].1.as_ref().unwrap().decl);
    let b_js = jsast::stmt_to_string(&lowered[1].1.as_ref().unwrap().decl);
    let sh_js = jsast::stmt_to_string(&lowered[2].1.as_ref().unwrap().decl);
    let wr_js = jsast::stmt_to_string(&lowered[3].1.as_ref().unwrap().decl);
    let hc_name = hc_items[0].name.as_str();
    assert!(
        !a_js.contains("new Uint8Array") && a_js.contains(&format!("{hc_name}[c] === 1")),
        "scan_a should index hoisted LUT: {a_js}"
    );
    assert!(
        !b_js.contains("new Uint8Array") && b_js.contains(&format!("{hc_name}[c] === 1")),
        "scan_b should share deduplicated LUT: {b_js}"
    );
    assert!(
        sh_js.contains("new Uint8Array(") && !sh_js.contains(hc_name),
        "shadowed Uint8Array param must NOT hoist: {sh_js}"
    );
    assert!(
        wr_js.contains("new Uint8Array(") && !wr_js.contains(hc_name),
        "assign-target LUT write must NOT hoist: {wr_js}"
    );
}
