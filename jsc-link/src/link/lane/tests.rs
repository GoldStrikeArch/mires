#![allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-item lane fixtures inline"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::diag::Sink;
use crate::item::{ItemKind, ItemMap, JsName, LinkItem, Linkage};
use crate::jir::decline::{Decline, LowerDecline, OpClass};
use crate::jir::summary::ItemClass;
use crate::jir::ItemId;
use crate::jsast;
use crate::lower::LoweredItem;
use crate::opts::Lane;

use super::driver::{
    apply, build_funcs, item_index, propagated_alloc_term, run, run_with_opts, vtable_index, Ctx,
};
use super::o_group::o_group_with_mode;
use super::report::{
    sidecar_path, LaneDisposition, LaneOutcome, LaneRun, PassCounts, Passes, ReprBlock,
    ReprDecision, ReprExport,
};

#[test]
fn a_transitively_hot_callee_charges_every_site_at_loop_depth_one() {
    assert_eq!(propagated_alloc_term(3, 72, true, 24), 288);
    assert_eq!(propagated_alloc_term(3, 1_200, true, 24), 1_200);
    assert_eq!(propagated_alloc_term(3, 72, false, 24), 72);
}

fn item(name: &str, refs: &[&str]) -> LinkItem {
    LinkItem {
        name: JsName::new(name.to_owned()),
        kind: ItemKind::Fn,
        js: format!("\nfunction {name}() {{\n  return 1;\n}}\n"),
        refs: refs
            .iter()
            .map(|name| JsName::new((*name).to_owned()))
            .collect(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: format!("test::{name}"),
        crate_name: "test".to_owned(),
        order: None,
        map: ItemMap::default(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    }
}

/// The declaration `function <name>() { return <callee>(); }`, and the refs it makes.
fn lowering(name: &str, callee: &str) -> LoweredItem {
    let decl = jsast::function(
        name.to_owned(),
        Vec::new(),
        vec![jsast::Stmt::Return(Some(jsast::call(
            jsast::Expr::Ident(callee.to_owned()),
            Vec::new(),
        )))],
    );
    LoweredItem {
        refs: [JsName::new(callee.to_owned())].into_iter().collect(),
        decl,
        names: Vec::new(),
        loc: None,
        dts: None,
        zombies: Vec::new(),
        js_alloc_origins: Vec::new(),
        bigints: BTreeSet::new(),
    }
}

/// An item with no decoded body is what a synthesized item looks like — the allocator shim,
/// a link-step stub — and it must never be offered, never counted, never a fallback.
#[test]
fn an_item_without_a_body_is_never_offered() {
    let dcx = Sink::new();
    let mut table = BTreeMap::new();
    let one = item("one$h1", &[]);
    table.insert(one.name.clone(), one);
    let reachable = vec![JsName::new("one$h1".to_owned())];
    let run = run(&dcx, &mut table, &reachable);
    assert_eq!(run.fallback_count(), 0);
    assert!(run.outcome(&JsName::new("one$h1".to_owned())).is_none());
    assert_eq!(dcx.errors(), 0);
}

#[test]
fn a_lowering_replaces_the_printed_text_and_the_map() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &["two$h2"]);
    let baseline = one.js.clone();
    let bytes = apply(
        &dcx,
        &mut one,
        lowering("one$h1", "two$h2"),
        &defined(&["one$h1", "two$h2"]),
        &defined(&["one$h1", "two$h2"]),
    )
    .expect("applied");
    assert_eq!(dcx.errors(), 0);
    assert_ne!(one.js, baseline);
    assert_eq!(one.js.len(), bytes.bytes);
    assert!(one.js.contains("return two$h2();"), "{}", one.js);
    // The header comment is the printer's, from the item's own debug path: the lane prints
    // through `JsItem::to_js_mapped`, exactly as the baseline was printed.
    assert!(one.js.starts_with("// test::one$h1\n"), "{}", one.js);
}

/// The item table the driver holds the two lanes to: the names a program defines.
fn defined(names: &[&str]) -> BTreeSet<JsName> {
    names
        .iter()
        .map(|name| JsName::new((*name).to_owned()))
        .collect()
}

#[test]
fn a_lowering_that_reaches_further_than_the_baseline_is_a_hard_error() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &["two$h2"]);
    let baseline = one.js.clone();
    let result = apply(
        &dcx,
        &mut one,
        lowering("one$h1", "three$h3"),
        &defined(&["one$h1", "two$h2", "three$h3"]),
        // `three$h3` is defined but was never reached, so nothing will print it.
        &defined(&["one$h1", "two$h2"]),
    );
    assert!(result.is_err());
    assert_eq!(dcx.errors(), 1);
    assert_eq!(one.js, baseline, "the item must keep its baseline text");
}

/// A lowering whose declared `refs` are not the ones its declaration makes is caught before
/// the baseline comparison: the two would otherwise cancel out.
#[test]
fn a_lowering_that_misdeclares_its_own_refs_is_a_hard_error() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &["two$h2"]);
    let mut lowered = lowering("one$h1", "two$h2");
    lowered.refs = BTreeSet::new();
    assert!(apply(
        &dcx,
        &mut one,
        lowered,
        &defined(&["one$h1", "two$h2"]),
        &defined(&["one$h1", "two$h2"])
    )
    .is_err());
    assert_eq!(dcx.errors(), 1);
}

/// A lowering that reaches FEWER items than the baseline is allowed: it proved something
/// dead, and a walk that already visited a superset has nothing to redo.
#[test]
fn a_lowering_may_reach_fewer_items_than_the_baseline() {
    let dcx = Sink::new();
    // The baseline calls `two$h2` and `three$h3`; the lowering folded the arm that called
    // `three$h3` away.
    let mut one = item("one$h1", &["two$h2", "three$h3"]);
    let bytes = apply(
        &dcx,
        &mut one,
        lowering("one$h1", "two$h2"),
        &defined(&["one$h1", "two$h2", "three$h3"]),
        &defined(&["one$h1", "two$h2", "three$h3"]),
    )
    .expect("applied");
    assert_eq!(dcx.errors(), 0);
    assert_eq!(one.js.len(), bytes.bytes);
    assert!(!one.js.contains("three$h3"), "{}", one.js);
    // The item's own refs are the ones its printed body makes, so the next walk sees them.
    assert!(!one.refs.contains(&JsName::new("three$h3".to_owned())));
    // And the walk has to run again, or `three$h3` stays in a program nothing calls it from.
    assert!(bytes.refs_changed);
}

/// The link step re-walks reachability when a lowering the lane KEPT reaches something else
/// than its baseline did. A row that was refused says nothing about the program: the item
/// kept its baseline text, which reaches what it always did.
#[test]
fn a_run_reports_a_reference_change_only_for_an_item_it_kept() {
    let row = |disposition| LaneOutcome {
        name: JsName::new("one$h1".to_owned()),
        class: ItemClass::empty(),
        disposition,
        bytes_baseline: 1,
        alloc_json: None,
        js_alloc_json: None,
        host_args: Default::default(),
        slice_iterator_would_lower: 0,
        slice_iterator_final_emitted: 0,
    };
    let run = |outcome: LaneOutcome| LaneRun {
        outcomes: BTreeMap::from([(outcome.name.clone(), outcome)]),
        ..LaneRun::default()
    };
    let fallback_decline = Decline::Lower(LowerDecline::Other {
        op: OpClass::Other,
        detail: "refused".to_owned(),
    });
    assert!(run(row(LaneDisposition::Jir {
        bytes_jir: 1,
        bytes_min: 1,
        refs_equal: false,
    }))
    .references_changed());
    assert!(!run(row(LaneDisposition::Jir {
        bytes_jir: 1,
        bytes_min: 1,
        refs_equal: true,
    }))
    .references_changed());
    assert!(!run(row(LaneDisposition::Fallback {
        decline: fallback_decline,
        refs_equal: false,
    }))
    .references_changed());
}

/// The JIR lowering of an item legitimately reaches names the baseline spelled another way —
/// `__rt`, the ESM shim import, is the one this rule was written for. Referencing an item the
/// walk already reached costs nothing: it is decoded, ordered and printed either way.
#[test]
fn a_lowering_may_reach_an_item_the_walk_already_prepared() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &[]);
    let bytes = apply(
        &dcx,
        &mut one,
        lowering("one$h1", "__rt"),
        &defined(&["one$h1", "__rt"]),
        &defined(&["one$h1", "__rt"]),
    )
    .expect("applied");
    assert_eq!(dcx.errors(), 0);
    assert_eq!(one.js.len(), bytes.bytes);
    assert!(one.refs.contains(&JsName::new("__rt".to_owned())));
}

/// A namer-shaped name no item defines is one `stub_missing` synthesizes a throwing body for,
/// so introducing one grows the program by an item. Refused like any other new reference.
#[test]
fn a_lowering_that_invents_a_namer_shaped_name_is_a_hard_error() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &["two$h2"]);
    let result = apply(
        &dcx,
        &mut one,
        lowering("one$h1", "never$h0000000000000009"),
        &defined(&["one$h1", "two$h2"]),
        &defined(&["one$h1", "two$h2"]),
    );
    assert!(result.is_err());
    assert_eq!(dcx.errors(), 1);
}

/// The two lanes name their temporaries differently, and that is the point of having two.
/// Only what the item table defines is held equal.
#[test]
fn a_lowering_may_name_its_locals_its_own_way() {
    let dcx = Sink::new();
    // The baseline named its temporary `_13`; the lowering names the same one `$v103`.
    let mut one = item("one$h1", &["two$h2", "_13"]);
    let mut lowered = lowering("one$h1", "two$h2");
    // `function one$h1() { let $v103 = 1; return two$h2($v103); }`
    let jsast::Stmt::FunctionDecl { body, .. } = &mut lowered.decl else {
        panic!("the lowering is a function declaration");
    };
    body.insert(0, jsast::Stmt::Let("$v103".to_owned(), Some(jsast::num(1))));
    *body.last_mut().expect("a return") = jsast::Stmt::Return(Some(jsast::call(
        jsast::Expr::Ident("two$h2".to_owned()),
        vec![jsast::Expr::Ident("$v103".to_owned())],
    )));
    let bytes = apply(
        &dcx,
        &mut one,
        lowered,
        &defined(&["one$h1", "two$h2"]),
        &defined(&["one$h1", "two$h2"]),
    )
    .expect("applied");
    assert_eq!(dcx.errors(), 0);
    assert!(one.js.contains("$v103"), "{}", one.js);
    assert_eq!(one.js.len(), bytes.bytes);
}

/// `$high` is a field lane suffix, not the readable namer's 16-hex hash suffix.
#[test]
fn a_physicalized_local_is_not_a_namer_shaped_reference() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &["two$h2"]);
    let mut lowered = lowering("one$h1", "two$h2");
    let jsast::Stmt::FunctionDecl { body, .. } = &mut lowered.decl else {
        panic!("the lowering is a function declaration");
    };
    body.insert(
        0,
        jsast::Stmt::Let("acc$high".to_owned(), Some(jsast::num(1))),
    );
    *body.last_mut().expect("a return") = jsast::Stmt::Return(Some(jsast::call(
        jsast::Expr::Ident("two$h2".to_owned()),
        vec![jsast::Expr::Ident("acc$high".to_owned())],
    )));

    apply(
        &dcx,
        &mut one,
        lowered,
        &defined(&["one$h1", "two$h2"]),
        &defined(&["one$h1", "two$h2"]),
    )
    .expect("applied");
    assert_eq!(dcx.errors(), 0);
}

#[test]
fn a_local_matching_an_unprepared_item_is_not_a_dependency() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &["two$h2"]);
    let mut lowered = lowering("one$h1", "two$h2");
    let jsast::Stmt::FunctionDecl { body, .. } = &mut lowered.decl else {
        panic!("a function declaration");
    };
    body.insert(0, jsast::let_("log", jsast::num(1)));
    *body.last_mut().expect("return") =
        jsast::ret(jsast::call(jsast::id("two$h2"), vec![jsast::id("log")]));
    apply(
        &dcx,
        &mut one,
        lowered,
        &defined(&["one$h1", "two$h2", "log"]),
        &defined(&["one$h1", "two$h2"]),
    )
    .expect("local is not an unprepared dependency");
    assert!(!one.refs.contains(&JsName::new("log".into())));
    assert_eq!(dcx.errors(), 0);
}

/// The lowerer produces no `.d.ts` yet, and `None` must not erase the one the object carried.
/// Nor may a lane touch what the item IS: its kind, its linkage, its declaration order.
#[test]
fn a_lowering_without_a_signature_keeps_the_baseline_one() {
    let dcx = Sink::new();
    let mut one = item("one$h1", &["two$h2"]);
    one.dts = Some(crate::dts::Dts {
        params: vec![("a".to_owned(), "number".to_owned())],
        ret: "number".to_owned(),
        types: Vec::new(),
        rust: "fn one(a: u32) -> u32".to_owned(),
    });
    apply(
        &dcx,
        &mut one,
        lowering("one$h1", "two$h2"),
        &defined(&["one$h1", "two$h2"]),
        &defined(&["one$h1", "two$h2"]),
    )
    .expect("applied");
    assert_eq!(
        one.dts.as_ref().map(|dts| dts.rust.as_str()),
        Some("fn one(a: u32) -> u32")
    );
    assert_eq!(one.kind, ItemKind::Fn);
    assert!(one.order.is_none());
}

#[test]
fn the_sidecar_is_the_shape_the_shadow_script_reads() {
    let run = LaneRun {
        passes: Passes {
            would_apply: PassCounts {
                sroa: 1,
                ast_dce: 2,
                inline: 1,
                devirt: 0,
            },
            applied: PassCounts {
                sroa: 1,
                ast_dce: 2,
                inline: 1,
                devirt: 0,
            },
        },
        abi_json: Some("{\"applied\":true,\"registers\":0}".to_owned()),
        repr: None,
        plan_retries: 0,
        added_items: false,
        outcomes: BTreeMap::from([
            (
                JsName::new("a$h1".to_owned()),
                LaneOutcome {
                    name: JsName::new("a$h1".to_owned()),
                    class: ItemClass::SCALAR_LEAF,
                    disposition: LaneDisposition::Jir {
                        bytes_jir: 36,
                        bytes_min: 28,
                        refs_equal: true,
                    },
                    bytes_baseline: 40,
                    alloc_json: Some("{\"materialize\":1}".to_owned()),
                    js_alloc_json: Some("{\"object\":1}".to_owned()),
                    host_args: Default::default(),
                    slice_iterator_would_lower: 2,
                    slice_iterator_final_emitted: 2,
                },
            ),
            (
                JsName::new("b$h2".to_owned()),
                LaneOutcome {
                    name: JsName::new("b$h2".to_owned()),
                    class: ItemClass::REFS.union(ItemClass::UNWIND),
                    disposition: LaneDisposition::Fallback {
                        decline: Decline::Lower(LowerDecline::Other {
                            op: OpClass::Other,
                            detail: "jir lowering not implemented".to_owned(),
                        }),
                        refs_equal: true,
                    },
                    bytes_baseline: 90,
                    alloc_json: None,
                    js_alloc_json: None,
                    host_args: Default::default(),
                    slice_iterator_would_lower: 1,
                    slice_iterator_final_emitted: 0,
                },
            ),
        ]),
        ..LaneRun::default()
    };
    assert_eq!(run.fallback_count(), 1);
    assert_eq!(
        run.render(Lane::Jir),
        "{\"schema\":17,\"lane\":\"jir\",\"fallback_count\":1,\
         \"passes\":{\"would_apply\":{\"sroa\":1,\"ast_dce\":2,\"inline\":1,\"devirt\":0},\
         \"applied\":{\"sroa\":1,\"ast_dce\":2,\"inline\":1,\"devirt\":0}},\
         \"summaries\":{\"items\":0,\"validated\":0,\"top\":0},\
         \"inline\":{\"applied\":0,\"cross_object_applied\":0,\"declined\":{},\"declined_budget\":0,\"bytes_before\":0,\"bytes_after\":0},\
         \"devirt\":{\"applied\":0},\
         \"range\":{\"asserts_removed\":0,\"comparisons_folded\":0},\
         \"slice_iter\":{\"would_lower\":3,\"final_emitted\":2},\
         \"witnesses\":{\"recorded\":0,\"failed\":0},\
         \"plan_retries\":0,\
         \"abi\":{\"applied\":true,\"registers\":0},\"physical\":null,\"async\":null,\
         \"repr\":null,\"items\":[\
         {\"name\":\"a$h1\",\"class\":[\"scalar-leaf\"],\"lane\":\"jir\",\"decline\":null,\
         \"refs_equal\":true,\"bytes_baseline\":40,\"bytes_jir\":36,\"bytes_min\":28,\
         \"slice_iter\":{\"would_lower\":2,\"final_emitted\":2},\
         \"host_args\":{\"declines\":{},\"final_emitted\":0,\"would_lower\":0},\
         \"alloc\":{\"materialize\":1},\"js_alloc\":{\"object\":1}},\
         {\"name\":\"b$h2\",\"class\":[\"refs\",\"unwind\"],\"lane\":\"fallback\",\
         \"decline\":\"lower.other\",\"refs_equal\":true,\"bytes_baseline\":90,\
         \"bytes_jir\":0,\"bytes_min\":0,\
         \"slice_iter\":{\"would_lower\":1,\"final_emitted\":0},\
         \"host_args\":{\"declines\":{},\"final_emitted\":0,\"would_lower\":0},\
         \"alloc\":null,\"js_alloc\":null}]}\n"
    );
}

/// The `repr` block renders D12's exact shape, decisions included, so the exit gate and
/// `scripts/repr-report-test.sh` read one stable schema.
#[test]
fn the_repr_block_is_the_shape_the_gate_reads() {
    let run = LaneRun {
        repr: Some(ReprBlock {
            families: 3,
            candidates: 4,
            legal: 3,
            cost_positive: 2,
            committed: 2,
            planned: 2,
            applied: 2,
            decisions: vec![
                ReprDecision {
                    subject: "a$h1/v3".to_owned(),
                    family: "string".to_owned(),
                    chosen: "host-string".to_owned(),
                    alternatives: vec!["host-string".to_owned(), "string-builder".to_owned()],
                    decline: None,
                    items: [JsName::new("a$h1".to_owned())].into_iter().collect(),
                },
                ReprDecision {
                    subject: "b$h2/v1".to_owned(),
                    family: "carrier".to_owned(),
                    chosen: "baseline".to_owned(),
                    alternatives: vec!["baseline".to_owned(), "native-vec".to_owned()],
                    decline: Some("cost.no-improvement before=120 after=140".to_owned()),
                    items: [JsName::new("b$h2".to_owned())].into_iter().collect(),
                },
            ],
            adapters: 1,
            versions: 1,
            helper_bytes: 512,
            pinned_cost_positive: 1,
            exports: vec![ReprExport {
                subject: "give$h3/ret".to_owned(),
                chosen: "host-string".to_owned(),
            }],
            bytes_before: 1_000,
            bytes_after: 900,
            score_before: 1_100,
            score_after: 950,
            emits: true,
        }),
        ..LaneRun::default()
    };
    let rendered = run.render(Lane::Jir);
    assert!(
        rendered.contains(
            "\"repr\":{\"families\":3,\"planned\":{\"candidates\":4,\"legal\":3,\
             \"would_commit\":2,\"pinned_cost_positive\":1},\
             \"validated\":{\"cost_positive\":2,\"would_commit\":2,\"adapters\":1,\
             \"versions\":1,\"helper_bytes\":512,\"bytes_before\":1000,\
             \"bytes_after\":900,\"score_before\":1100,\"score_after\":950},\
             \"final_emitted\":{\"applied\":2,\"bytes\":900},\"decisions\":[\
             {\"subject\":\"a$h1/v3\",\"family\":\"string\",\"chosen\":\"host-string\",\
             \"alternatives\":[\"host-string\",\"string-builder\"],\"decline\":null},\
             {\"subject\":\"b$h2/v1\",\"family\":\"carrier\",\"chosen\":\"baseline\",\
             \"alternatives\":[\"baseline\",\"native-vec\"],\
             \"decline\":\"cost.no-improvement before=120 after=140\"}],\
             \"exports\":[{\"subject\":\"give$h3/ret\",\"chosen\":\"host-string\"}]}"
        ),
        "{rendered}"
    );
}

/// `link_dedup` may retire an item the lane lowered; the sidecar describes what was printed.
#[test]
fn a_row_whose_item_stopped_being_printed_is_dropped() {
    let gone = JsName::new("gone$h1".to_owned());
    let mut run = LaneRun {
        outcomes: BTreeMap::from([(
            gone.clone(),
            LaneOutcome {
                name: gone,
                class: ItemClass::empty(),
                disposition: LaneDisposition::Jir {
                    bytes_jir: 1,
                    bytes_min: 1,
                    refs_equal: true,
                },
                bytes_baseline: 1,
                alloc_json: None,
                js_alloc_json: None,
                host_args: Default::default(),
                slice_iterator_would_lower: 1,
                slice_iterator_final_emitted: 1,
            },
        )]),
        ..LaneRun::default()
    };
    run.retain_reachable(&[JsName::new("other$h2".to_owned())]);
    assert!(run.render(Lane::Jir).contains("\"items\":[]"));
}

#[test]
fn rename_keeps_outcome_lookup_and_render_order_valid() {
    let make = |name: &str| {
        let key = JsName::new(name.to_owned());
        (
            key.clone(),
            LaneOutcome {
                name: key,
                class: ItemClass::empty(),
                disposition: LaneDisposition::Jir {
                    bytes_jir: 10,
                    bytes_min: 8,
                    refs_equal: true,
                },
                bytes_baseline: 12,
                alloc_json: None,
                js_alloc_json: None,
                host_args: Default::default(),
                slice_iterator_would_lower: 0,
                slice_iterator_final_emitted: 0,
            },
        )
    };
    let mut run = LaneRun {
        outcomes: BTreeMap::from([make("a$h1"), make("m$h2")]),
        ..LaneRun::default()
    };
    let from = JsName::new("a$h1".to_owned());
    let to = JsName::new("z$h1".to_owned());
    run.rename(&from, &to);
    assert!(run.outcome(&from).is_none());
    let moved = run
        .outcome(&to)
        .expect("renamed outcome must remain findable");
    assert_eq!(moved.name, to);
    assert!(run.outcome(&JsName::new("m$h2".to_owned())).is_some());
    let rendered = run.render(Lane::Jir);
    let m_pos = rendered.find("\"name\":\"m$h2\"").expect("m$h2 present");
    let z_pos = rendered.find("\"name\":\"z$h1\"").expect("z$h1 present");
    assert!(m_pos < z_pos, "rendered items must remain sorted by name");
}

#[test]
fn the_sidecar_path_sits_beside_the_program() {
    let path = Path::new("/build/program.js");
    assert_eq!(sidecar_path(path), Path::new("/build/program.js.lane.json"));
}

#[test]
fn abi_fault_retry_matches_flat_abi_off_with_post_repr_vec_push() {
    use crate::jir::origin::{Origin, OriginKind, OriginTable};
    use crate::jir::semantic::{
        AbiSignature, AggregateKind, BasicBlock, BinaryOp, BlockId, Body, BodyKind, BodySemantic,
        Call, CallSemantic, Callee, Constant, DefinitionId, DirectItemRef, Local, LocalId,
        LocalRole, Operand, OperandKind, Place, Projection, Rvalue, Signature, StandardOperation,
        Statement, StatementKind, Terminator, Unwind,
    };
    use crate::jir::types::{
        FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor, TypeId,
        TypeKind, TypeMeta,
    };
    use crate::jir::{Interns, OriginId};
    use crate::opts::{Lane, OptimizerMode, Opts};
    use rcgjs_object::item::{
        read_object, write_object, JirItem, JsItem, JsItemJir, ObjectIdentity,
    };

    const U32: TypeId = TypeId(1);
    const PAIR: TypeId = TypeId(2);
    const FN_FLAT: TypeId = TypeId(3);
    const FN_VEC_PUSH: TypeId = TypeId(4);

    let fn_item = |local_hash: u64| ItemId {
        definition: DefinitionId {
            crate_id: 0,
            local_hash,
        },
        instantiation: u128::from(local_hash),
    };
    let flat_id = fn_item(2);
    let push_id = fn_item(3);

    let mut interns = Interns::new();
    let flat_name = interns.intern_str("flat_callee$h2");
    let push_name = interns.intern_str("vec_push$h3");
    let empty_js_facts = |peeled: TypeId| JsFacts {
        int: None,
        is_indirect: false,
        needs_clone: false,
        crosses_transparent_union: false,
        transparent: None,
        peeled,
        unsized_tail: None,
        enum_repr: None,
        is_primitive_enum: false,
        pointer_form: None,
        byte_element: false,
        fields: Vec::new(),
    };
    let u32_desc = TypeDescriptor {
        id: U32,
        name: None,
        kind: TypeKind::Integer {
            signed: false,
            bits: 32,
        },
        layout: LayoutFacts {
            size: Some(4),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            int: Some(IntFacts {
                signed: false,
                bits: 32,
                repr: IntRepr::Number,
            }),
            ..empty_js_facts(U32)
        },
    };
    let pair_desc = TypeDescriptor {
        id: PAIR,
        name: None,
        kind: TypeKind::Tuple(vec![U32, U32]),
        layout: LayoutFacts {
            size: Some(8),
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: JsFacts {
            is_indirect: true,
            needs_clone: true,
            fields: vec![
                FieldDescriptor {
                    key: FieldKey::Index(0),
                    ty: U32,
                    offset: 0,
                },
                FieldDescriptor {
                    key: FieldKey::Index(1),
                    ty: U32,
                    offset: 4,
                },
            ],
            ..empty_js_facts(PAIR)
        },
    };
    let fn_desc = |id: TypeId, local_hash: u64, item: ItemId, link_name| TypeDescriptor {
        id,
        name: None,
        kind: TypeKind::FnDef {
            def: DefinitionId {
                crate_id: 0,
                local_hash,
            },
            sig: crate::jir::interns::SigId(0),
            instance: Some(DirectItemRef { item, link_name }),
        },
        layout: LayoutFacts {
            size: Some(0),
            zst: true,
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: empty_js_facts(id),
    };
    for desc in [
        u32_desc,
        pair_desc,
        fn_desc(FN_FLAT, 2, flat_id, flat_name),
        fn_desc(FN_VEC_PUSH, 3, push_id, push_name),
    ] {
        interns.types.insert(desc);
    }

    let place = |local: u32, ty: TypeId, proj: Vec<Projection>| Place {
        local: LocalId(local),
        projection: proj,
        ty,
    };
    let copy = |local: u32, ty: TypeId| Operand {
        ty,
        kind: OperandKind::Copy(place(local, ty, Vec::new())),
    };
    let u32_const = |v: u128| Operand {
        ty: U32,
        kind: OperandKind::Constant(Constant::Scalar { bits: v, bytes: 4 }),
    };
    let assign = |dest: u32, ty: TypeId, value: Rvalue| Statement {
        kind: StatementKind::Assign {
            destination: place(dest, ty, Vec::new()),
            value,
        },
    };

    // `vec_push$h3(x: u32) -> u32`: `CallSemantic::Standard(VecLen)` defers pre-repr
    // inlining so it only inlines during post-repr `o_group_with_mode(..., true)`.
    let vec_push_body = Body {
        item: push_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Standard(StandardOperation::VecLen),
        signature: Signature {
            arguments: vec![U32],
            result: U32,
            abi: AbiSignature {
                parameters: 1,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
        },
        locals: vec![
            Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: U32,
            },
            Local {
                id: LocalId(1),
                role: LocalRole::Argument,
                ty: U32,
            },
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements: vec![assign(
                0,
                U32,
                Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: copy(1, U32),
                    right: u32_const(1),
                },
            )],
            terminator: Terminator::Return,
        }],
    };

    // `flat_callee$h2(p: (u32, u32)) -> u32`: big enough not to inline across 4 sites, small
    // enough for `abi_flat` to flatten `p` into two `u32` arguments.
    let mut flat_stmts = vec![
        assign(
            2,
            U32,
            Rvalue::Use(Operand {
                ty: U32,
                kind: OperandKind::Copy(place(
                    1,
                    U32,
                    vec![Projection::Field { field: 0, ty: U32 }],
                )),
            }),
        ),
        assign(
            3,
            U32,
            Rvalue::Use(Operand {
                ty: U32,
                kind: OperandKind::Copy(place(
                    1,
                    U32,
                    vec![Projection::Field { field: 1, ty: U32 }],
                )),
            }),
        ),
        assign(
            4,
            U32,
            Rvalue::Binary {
                op: BinaryOp::Add,
                left: copy(2, U32),
                right: copy(3, U32),
            },
        ),
    ];
    for step in 1..22u128 {
        flat_stmts.push(assign(
            4,
            U32,
            Rvalue::Binary {
                op: BinaryOp::BitXor,
                left: copy(4, U32),
                right: u32_const(step),
            },
        ));
    }
    flat_stmts.push(assign(0, U32, Rvalue::Use(copy(4, U32))));
    let flat_body = Body {
        item: flat_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: Signature {
            arguments: vec![PAIR],
            result: U32,
            abi: AbiSignature {
                parameters: 1,
                spread: None,
                caller_location: false,
                inline_never: false,
                inline_hint: false,
                inline_always: false,
            },
        },
        locals: vec![
            Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: U32,
            },
            Local {
                id: LocalId(1),
                role: LocalRole::Argument,
                ty: PAIR,
            },
            Local {
                id: LocalId(2),
                role: LocalRole::Temporary,
                ty: U32,
            },
            Local {
                id: LocalId(3),
                role: LocalRole::Temporary,
                ty: U32,
            },
            Local {
                id: LocalId(4),
                role: LocalRole::Temporary,
                ty: U32,
            },
        ],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements: flat_stmts,
            terminator: Terminator::Return,
        }],
    };

    // `caller$h1(x: u32) -> u32`: calls `vec_push$h3(x)`, then calls `flat_callee$h2` 4 times
    // with freshly constructed `(u32, u32)` pairs so `abi_flat` accepts `flat_callee$h2`.
    let caller_body = {
        let mut blocks = Vec::new();
        blocks.push(BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements: Vec::new(),
            terminator: Terminator::Call(Call {
                function: Operand {
                    ty: FN_VEC_PUSH,
                    kind: OperandKind::Constant(Constant::ZeroSized),
                },
                callee: Callee::Direct {
                    item: push_id,
                    link_name: push_name,
                },
                semantic: CallSemantic::Standard(StandardOperation::VecLen),
                arguments: vec![copy(1, U32)],
                destination: place(2, U32, Vec::new()),
                target: Some(BlockId(1)),
                unwind: Unwind::Continue,
                track_caller: None,
                host: None,
            }),
        });
        for idx in 0..4u32 {
            let pair_local = 3 + idx * 2;
            let res_local = 4 + idx * 2;
            blocks.push(BasicBlock {
                id: BlockId(1 + idx),
                cleanup: false,
                statements: vec![assign(
                    pair_local,
                    PAIR,
                    Rvalue::Aggregate {
                        kind: AggregateKind::Tuple,
                        result: PAIR,
                        operands: vec![copy(2, U32), u32_const(u128::from(idx))],
                    },
                )],
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: FN_FLAT,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    callee: Callee::Direct {
                        item: flat_id,
                        link_name: flat_name,
                    },
                    semantic: CallSemantic::Ordinary,
                    arguments: vec![copy(pair_local, PAIR)],
                    destination: place(res_local, U32, Vec::new()),
                    target: Some(BlockId(2 + idx)),
                    unwind: Unwind::Continue,
                    track_caller: None,
                    host: None,
                }),
            });
        }
        blocks.push(BasicBlock {
            id: BlockId(5),
            cleanup: false,
            statements: vec![
                assign(
                    11,
                    U32,
                    Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: copy(4, U32),
                        right: copy(6, U32),
                    },
                ),
                assign(
                    12,
                    U32,
                    Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: copy(8, U32),
                        right: copy(10, U32),
                    },
                ),
                assign(
                    0,
                    U32,
                    Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: copy(11, U32),
                        right: copy(12, U32),
                    },
                ),
            ],
            terminator: Terminator::Return,
        });
        let mut locals = vec![
            Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: U32,
            },
            Local {
                id: LocalId(1),
                role: LocalRole::Argument,
                ty: U32,
            },
            Local {
                id: LocalId(2),
                role: LocalRole::Temporary,
                ty: U32,
            },
        ];
        for idx in 0..4u32 {
            locals.push(Local {
                id: LocalId(3 + idx * 2),
                role: LocalRole::Temporary,
                ty: PAIR,
            });
            locals.push(Local {
                id: LocalId(4 + idx * 2),
                role: LocalRole::Temporary,
                ty: U32,
            });
        }
        locals.push(Local {
            id: LocalId(11),
            role: LocalRole::Temporary,
            ty: U32,
        });
        locals.push(Local {
            id: LocalId(12),
            role: LocalRole::Temporary,
            ty: U32,
        });
        Body {
            item: fn_item(1),
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![U32],
                result: U32,
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
            },
            locals,
            blocks,
        }
    };

    let body_origins = |item_path, body: &Body| {
        let stmt_count: usize = body.blocks.iter().map(|b| b.statements.len()).sum();
        let make_origin = |kind| Origin {
            kind,
            source: None,
            local_name: None,
            inline_stack: Vec::new(),
        };
        OriginTable {
            origins: vec![
                make_origin(OriginKind::Statement),
                make_origin(OriginKind::Terminator),
                make_origin(OriginKind::Local),
            ],
            statements: vec![OriginId(0); stmt_count],
            terminators: vec![OriginId(1); body.blocks.len()],
            locals: vec![OriginId(2); body.locals.len()],
            item_path,
            root: None,
        }
    };
    let stub_decl =
        |name: &str| jsast::function(name.to_owned(), Vec::new(), vec![jsast::Stmt::Return(None)]);
    let caller_name_str = interns.intern_str("caller$h1");
    let caller_decl = jsast::function(
        "caller$h1".to_owned(),
        vec!["x".to_owned()],
        vec![jsast::Stmt::Return(Some(jsast::binary(
            jsast::BinOp::Add,
            jsast::call(
                jsast::Expr::Ident("vec_push$h3".to_owned()),
                vec![jsast::Expr::Ident("x".to_owned())],
            ),
            jsast::call(
                jsast::Expr::Ident("flat_callee$h2".to_owned()),
                vec![jsast::Expr::Array(vec![])],
            ),
        )))],
    );

    let caller_origins = body_origins(caller_name_str, &caller_body);
    let flat_origins = body_origins(flat_name, &flat_body);
    let push_origins = body_origins(push_name, &vec_push_body);

    let js_items = vec![
        JsItem::new(
            JsName::new("caller$h1".to_owned()),
            ItemKind::Fn,
            caller_decl,
            Vec::new(),
            Linkage::exported("caller$h1".to_owned()),
            "test::caller$h1".to_owned(),
        )
        .with_jir(JsItemJir::from(JirItem {
            body: caller_body,
            origins: caller_origins,
        })),
        JsItem::new(
            JsName::new("flat_callee$h2".to_owned()),
            ItemKind::Fn,
            stub_decl("flat_callee$h2"),
            Vec::new(),
            Linkage::internal(),
            "test::flat_callee$h2".to_owned(),
        )
        .with_jir(JsItemJir::from(JirItem {
            body: flat_body,
            origins: flat_origins,
        })),
        JsItem::new(
            JsName::new("vec_push$h3".to_owned()),
            ItemKind::Fn,
            stub_decl("vec_push$h3"),
            Vec::new(),
            Linkage::internal(),
            "test::vec_push$h3".to_owned(),
        )
        .with_jir(JsItemJir::from(JirItem {
            body: vec_push_body,
            origins: push_origins,
        })),
    ];

    let obj_bytes = write_object(
        "",
        &js_items,
        true,
        "test",
        &ObjectIdentity::default(),
        interns,
        &BTreeMap::new(),
    );
    let read = read_object(&obj_bytes)
        .expect("valid object")
        .expect("format 20 object");
    let mut pristine_table = BTreeMap::new();
    for mut entry in read.items {
        assert!(entry.jir.decode().expect("decode jir"));
        pristine_table.insert(entry.name.clone(), entry);
    }

    let reachable = vec![
        JsName::new("caller$h1".to_owned()),
        JsName::new("flat_callee$h2".to_owned()),
        JsName::new("vec_push$h3".to_owned()),
    ];
    let dcx = Sink::default();

    let opts_on = Opts {
        optimizer: OptimizerMode::On,
        lane: Lane::Jir,
        flat_abi: true,
        ..Opts::default()
    };
    let mut fault_table = pristine_table.clone();
    let fault_run = run_with_opts(
        &dcx,
        &mut fault_table,
        &reachable,
        &opts_on,
        Some("flat_callee"),
    );

    let opts_flat_off = Opts {
        optimizer: OptimizerMode::On,
        lane: Lane::Jir,
        flat_abi: false,
        ..Opts::default()
    };
    let mut off_table = pristine_table;
    let off_run = run_with_opts(&dcx, &mut off_table, &reachable, &opts_flat_off, None);

    assert_eq!(
        fault_run.plan_retries, 1,
        "abi_json={:?}, passes={:?}, outcomes={:?}",
        fault_run.abi_json, fault_run.passes, fault_run.outcomes
    );
    assert_eq!(off_run.plan_retries, 0);
    assert!(
        fault_run.passes.applied.inline >= 1,
        "post-repr inline must inline vec_push after retry: passes={:?}, o={:?}, caller_js={}",
        fault_run.passes,
        fault_run.o,
        fault_table[&JsName::new("caller$h1".to_owned())].js
    );
    assert_eq!(
        fault_run.passes.applied.inline,
        off_run.passes.applied.inline
    );

    let caller_name = JsName::new("caller$h1".to_owned());
    assert!(
        !fault_table[&caller_name].js.contains("vec_push"),
        "vec_push must be inlined into caller on retry: {}",
        fault_table[&caller_name].js
    );
    for (name, off_item) in &off_table {
        assert_eq!(
            fault_table[name].js, off_item.js,
            "item {name} JS diverged between ABI-fault retry and flat_abi=off"
        );
    }
}

#[test]
fn o_group_prunes_invoke_after_inlining_noop_wrapper_and_in_post_repr_mode() {
    use crate::jir::origin::{Origin, OriginKind, OriginTable};
    use crate::jir::semantic::{
        AbiSignature, BasicBlock, BlockId, Body, BodyKind, BodySemantic, Call, CallSemantic,
        Callee, DefinitionId, DirectItemRef, HostPayload, Local, LocalId, LocalRole, Operand,
        OperandKind, Place, Signature, Terminator, Unwind,
    };
    use crate::jir::types::{JsFacts, LayoutFacts, TypeDescriptor, TypeId, TypeKind, TypeMeta};
    use crate::jir::{Interns, OriginId};
    use crate::opts::{Lane, OptimizerMode, Opts};
    use rcgjs_object::item::{
        read_object, write_object, JirItem, JsItem, JsItemJir, ObjectIdentity,
    };

    const UNIT: TypeId = TypeId(1);
    const FN_EMPTY: TypeId = TypeId(2);
    const FN_WRAP: TypeId = TypeId(3);
    const FN_HOST: TypeId = TypeId(4);

    let fn_item = |local_hash: u64| ItemId {
        definition: DefinitionId {
            crate_id: 0,
            local_hash,
        },
        instantiation: u128::from(local_hash),
    };
    let caller_id = fn_item(1);
    let wrapper_id = fn_item(2);
    let empty_id = fn_item(3);

    let mut interns = Interns::new();
    let caller_str = interns.intern_str("caller$h1");
    let wrapper_str = interns.intern_str("wrapper$h2");
    let empty_str = interns.intern_str("empty_drop$h3");
    let host_sec = interns.intern_str("rcgjs.host.isnull");

    let empty_js_facts = |peeled: TypeId| JsFacts {
        int: None,
        is_indirect: false,
        needs_clone: false,
        crosses_transparent_union: false,
        transparent: None,
        peeled,
        unsized_tail: None,
        enum_repr: None,
        is_primitive_enum: false,
        pointer_form: None,
        byte_element: false,
        fields: Vec::new(),
    };
    interns.types.insert(TypeDescriptor {
        id: UNIT,
        name: None,
        kind: TypeKind::Tuple(Vec::new()),
        layout: LayoutFacts {
            size: Some(0),
            zst: true,
            sized: true,
            ..LayoutFacts::default()
        },
        meta: TypeMeta::default(),
        js: empty_js_facts(UNIT),
    });
    for (ty_id, h, item_id, name) in [
        (FN_EMPTY, 3, empty_id, empty_str),
        (FN_WRAP, 2, wrapper_id, wrapper_str),
        (
            FN_HOST,
            4,
            fn_item(4),
            interns.intern_str("isnull_marker$h4"),
        ),
    ] {
        interns.types.insert(TypeDescriptor {
            id: ty_id,
            name: None,
            kind: TypeKind::FnDef {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash: h,
                },
                sig: crate::jir::interns::SigId(0),
                instance: Some(DirectItemRef {
                    item: item_id,
                    link_name: name,
                }),
            },
            layout: LayoutFacts {
                size: Some(0),
                zst: true,
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: empty_js_facts(ty_id),
        });
    }

    let unit_sig = Signature {
        arguments: Vec::new(),
        result: UNIT,
        abi: AbiSignature::default(),
    };
    let unit_ret_local = Local {
        id: LocalId(0),
        role: LocalRole::Return,
        ty: UNIT,
    };
    let unit_place = Place {
        local: LocalId(0),
        projection: Vec::new(),
        ty: UNIT,
    };

    // `empty_drop$h3() { return; }`
    let empty_body = Body {
        item: empty_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: unit_sig.clone(),
        locals: vec![unit_ret_local],
        blocks: vec![BasicBlock {
            id: BlockId(0),
            cleanup: false,
            statements: Vec::new(),
            terminator: Terminator::Return,
        }],
    };

    // `wrapper$h2() { empty_drop$h3(); return; }` — non-empty before `inline`, becomes
    // 0-op `Return(None)` after `inline` splices `empty_drop$h3`.
    let wrapper_body = Body {
        item: wrapper_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: unit_sig.clone(),
        locals: vec![unit_ret_local],
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: FN_EMPTY,
                        kind: OperandKind::Constant(crate::jir::semantic::Constant::ZeroSized),
                    },
                    callee: Callee::Direct {
                        item: empty_id,
                        link_name: empty_str,
                    },
                    semantic: CallSemantic::Ordinary,
                    arguments: Vec::new(),
                    destination: unit_place.clone(),
                    target: Some(BlockId(1)),
                    unwind: Unwind::Continue,
                    track_caller: None,
                    host: None,
                }),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            },
        ],
    };

    // `caller$h1()` invokes a throwing host marker; its unwind cleanup calls `wrapper$h2` and
    // rethrows (`UnwindResume`). Only after `inline` empties `wrapper$h2` does post-inline
    // `prune_invoke::run` discover `wrapper$h2` is a noop and prune `caller`'s `Invoke`.
    let caller_body = Body {
        item: caller_id,
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: unit_sig,
        locals: vec![unit_ret_local],
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: FN_HOST,
                        kind: OperandKind::Constant(crate::jir::semantic::Constant::ZeroSized),
                    },
                    callee: Callee::Direct {
                        item: fn_item(4),
                        link_name: empty_str,
                    },
                    semantic: CallSemantic::Host,
                    arguments: Vec::new(),
                    destination: unit_place.clone(),
                    target: Some(BlockId(1)),
                    unwind: Unwind::Cleanup(BlockId(2)),
                    track_caller: None,
                    host: Some(HostPayload::Marker {
                        section: host_sec,
                        def: DefinitionId {
                            crate_id: 0,
                            local_hash: 4,
                        },
                        helpers: Vec::new(),
                    }),
                }),
            },
            BasicBlock {
                id: BlockId(1),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            },
            BasicBlock {
                id: BlockId(2),
                cleanup: true,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: FN_WRAP,
                        kind: OperandKind::Constant(crate::jir::semantic::Constant::ZeroSized),
                    },
                    callee: Callee::Direct {
                        item: wrapper_id,
                        link_name: wrapper_str,
                    },
                    semantic: CallSemantic::Ordinary,
                    arguments: Vec::new(),
                    destination: unit_place,
                    target: Some(BlockId(3)),
                    unwind: Unwind::Terminate(crate::jir::semantic::TerminateReason::InCleanup),
                    track_caller: None,
                    host: None,
                }),
            },
            BasicBlock {
                id: BlockId(3),
                cleanup: true,
                statements: Vec::new(),
                terminator: Terminator::UnwindResume,
            },
        ],
    };

    let make_origins = |item_path, body: &Body| OriginTable {
        origins: vec![
            Origin {
                kind: OriginKind::Terminator,
                source: None,
                local_name: None,
                inline_stack: Vec::new(),
            },
            Origin {
                kind: OriginKind::Local,
                source: None,
                local_name: None,
                inline_stack: Vec::new(),
            },
        ],
        statements: Vec::new(),
        terminators: vec![OriginId(0); body.blocks.len()],
        locals: vec![OriginId(1); body.locals.len()],
        item_path,
        root: None,
    };
    let stub_decl =
        |name: &str| jsast::function(name.to_owned(), Vec::new(), vec![jsast::Stmt::Return(None)]);
    let caller_decl = jsast::function(
        "caller$h1".to_owned(),
        Vec::new(),
        vec![jsast::Stmt::Return(Some(jsast::call(
            jsast::Expr::Ident("wrapper$h2".to_owned()),
            Vec::new(),
        )))],
    );
    let wrapper_decl = jsast::function(
        "wrapper$h2".to_owned(),
        Vec::new(),
        vec![jsast::Stmt::Return(Some(jsast::call(
            jsast::Expr::Ident("empty_drop$h3".to_owned()),
            Vec::new(),
        )))],
    );
    let caller_origins = make_origins(caller_str, &caller_body);
    let wrapper_origins = make_origins(wrapper_str, &wrapper_body);
    let empty_origins = make_origins(empty_str, &empty_body);

    let js_items = vec![
        JsItem::new(
            JsName::new("caller$h1".to_owned()),
            ItemKind::Fn,
            caller_decl,
            Vec::new(),
            Linkage::exported("caller$h1".to_owned()),
            "test::caller$h1".to_owned(),
        )
        .with_jir(JsItemJir::from(JirItem {
            body: caller_body,
            origins: caller_origins,
        })),
        JsItem::new(
            JsName::new("wrapper$h2".to_owned()),
            ItemKind::Fn,
            wrapper_decl,
            Vec::new(),
            Linkage::internal(),
            "test::wrapper$h2".to_owned(),
        )
        .with_jir(JsItemJir::from(JirItem {
            body: wrapper_body,
            origins: wrapper_origins,
        })),
        JsItem::new(
            JsName::new("empty_drop$h3".to_owned()),
            ItemKind::Fn,
            stub_decl("empty_drop$h3"),
            Vec::new(),
            Linkage::internal(),
            "test::empty_drop$h3".to_owned(),
        )
        .with_jir(JsItemJir::from(JirItem {
            body: empty_body,
            origins: empty_origins,
        })),
    ];
    let obj_bytes = write_object(
        "",
        &js_items,
        true,
        "test",
        &ObjectIdentity::default(),
        interns,
        &BTreeMap::new(),
    );
    let read = read_object(&obj_bytes).unwrap().unwrap();
    let mut table = BTreeMap::new();
    for mut entry in read.items {
        assert!(entry.jir.decode().unwrap());
        table.insert(entry.name.clone(), entry);
    }

    let opts = Opts {
        optimizer: OptimizerMode::On,
        lane: Lane::Jir,
        ..Opts::default()
    };
    let (items, drop_glue) = item_index(&table);
    let vtables = vtable_index(&table);
    let heap = crate::analysis::heap::HeapPlan::default();
    let ctx = Ctx {
        opts: &opts,
        items: &items,
        drop_glue: &drop_glue,
        vtables: &vtables,
        byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
        heap: &heap,
    };
    let names = vec![
        JsName::new("caller$h1".to_owned()),
        JsName::new("wrapper$h2".to_owned()),
        JsName::new("empty_drop$h3".to_owned()),
    ];
    let live: BTreeSet<JsName> = names.iter().cloned().collect();
    let mut funcs = build_funcs(&table, &names, &ctx);
    assert!(funcs[&JsName::new("caller$h1".to_owned())]
        .as_ref()
        .unwrap()
        .has_invoke());

    let group = o_group_with_mode(&mut table, &mut funcs, &ctx, &live, true);
    assert_eq!(group.inline.applied, 1);
    assert!(
        !funcs[&JsName::new("caller$h1".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke(),
        "post-inline prune_invoke must prune Invoke once inline empties wrapper$h2 (even with allow_std_post_repr=true)"
    );
}
