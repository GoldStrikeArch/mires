use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::process::{Command, Stdio};

use rcgjs_object::jsast::{self as js, ArrowBody, Expr, Loc, Stmt, Style};
use rcgjs_object::opts::{EcmaTarget, OptimizerMode};
use rcgjs_object::{minify, optimizer, sroa};

fn catching(body: Vec<Stmt>, binding: &str, handler: Vec<Stmt>) -> Stmt {
    Stmt::TryCatch {
        body,
        binding: binding.to_owned(),
        handler,
    }
}

fn invoke(name: &str, args: Vec<Expr>) -> Expr {
    js::call(js::id(name), args)
}

fn fixture() -> Vec<Stmt> {
    let cleanup = catching(
        vec![
            js::assign_stmt(
                js::id("state"),
                js::array(vec![
                    invoke("step", vec![js::num(1)]),
                    invoke("step", vec![js::num(2)]),
                ]),
            ),
            js::expr_stmt(invoke("step", vec![js::num(3)])),
            js::ret(js::id("state")),
        ],
        "caught",
        vec![
            js::expr_stmt(invoke("observe", vec![js::id("state")])),
            js::throw(js::id("caught")),
        ],
    );
    vec![
        js::function(
            "entry",
            vec![],
            vec![
                js::let_("state", js::array(vec![js::num(0)])),
                catching(
                    vec![cleanup],
                    "outer",
                    vec![
                        js::expr_stmt(invoke("outerCleanup", vec![])),
                        js::throw(js::id("outer")),
                    ],
                ),
            ],
        ),
        js::function(
            "shadow",
            vec!["argument".to_owned()],
            vec![
                Stmt::Let("saved".to_owned(), None),
                catching(
                    vec![js::throw(js::id("argument"))],
                    "caught",
                    vec![js::assign_stmt(
                        js::id("saved"),
                        Expr::Arrow(vec![], ArrowBody::Expr(Box::new(js::id("caught")))),
                    )],
                ),
                js::ret(js::array(vec![js::id("caught"), invoke("saved", vec![])])),
            ],
        ),
        js::function(
            "loopRecovery",
            vec![],
            vec![
                js::let_("count", js::num(0)),
                js::labeled(
                    "outerLoop",
                    js::while_(
                        js::binary(js::BinOp::Lt, js::id("count"), js::num(3)),
                        vec![
                            js::expr_stmt(js::compound_assign(
                                js::BinOp::Add,
                                js::id("count"),
                                js::num(1),
                            )),
                            catching(
                                vec![
                                    js::if_(
                                        js::binary(js::BinOp::Lt, js::id("count"), js::num(3)),
                                        vec![js::throw(js::id("count"))],
                                    ),
                                    Stmt::Break(Some("outerLoop".to_owned())),
                                ],
                                "loopError",
                                vec![
                                    js::expr_stmt(invoke("observeLoop", vec![js::id("loopError")])),
                                    Stmt::Continue(Some("outerLoop".to_owned())),
                                ],
                            ),
                        ],
                    ),
                ),
                js::ret(js::id("count")),
            ],
        ),
        js::function(
            "parameterShadow",
            vec!["caught".to_owned()],
            vec![
                catching(
                    vec![js::throw(js::num(9))],
                    "caught",
                    vec![js::expr_stmt(invoke("checkNine", vec![js::id("caught")]))],
                ),
                js::ret(js::id("caught")),
            ],
        ),
    ]
}

#[test]
fn catch_preserves_prefix_writes_scope_and_thrown_identity_through_the_ast_pipeline() {
    // All host injection lives in the harness. The tested function bodies contain structured AST.
    let harness = r#"
const assert = require('node:assert/strict');
const caught = {};
let trace, failAt, thrown, cleanupThrows, cleanupError;
function step(n) { trace.push(n); if (n === failAt) throw thrown; return n; }
function observe(state) { trace.push(['inner', ...state]); if (cleanupThrows) throw cleanupError; }
function outerCleanup() { trace.push('outer'); }
function checkNine(value) { assert.equal(value, 9); }
const values = [undefined, null, false, 0, -0, NaN, 'error', Symbol('error'), {}, new Error('error')];
let checks = 0;
for (thrown of values) for (failAt of [0, 1, 2, 3]) for (cleanupThrows of [false, true]) {
  trace = []; cleanupError = {}; let result, error, failed = false;
  try { result = entry(); } catch (e) { failed = true; error = e; }
  if (failAt === 0) {
    assert.equal(failed, false); assert.deepEqual(result, [1, 2]); assert.deepEqual(trace, [1, 2, 3]);
  } else {
    assert.equal(failed, true); assert.ok(Object.is(error, cleanupThrows ? cleanupError : thrown));
    assert.deepEqual(trace, [...Array.from({length: failAt}, (_, i) => i + 1), ['inner', ...(failAt === 3 ? [1, 2] : [0])], 'outer']);
  }
  const shadowed = shadow(thrown);
  assert.equal(shadowed[0], caught); assert.ok(Object.is(shadowed[1], thrown));
  assert.ok(Object.is(parameterShadow(thrown), thrown)); checks++;
}
const loopTrace = [];
function observeLoop(value) { loopTrace.push(value); }
assert.equal(loopRecovery(), 3); assert.deepEqual(loopTrace, [1, 2]);
console.log(checks);
"#;
    for jir in [false, true] {
        for mode in [OptimizerMode::Off, OptimizerMode::Report, OptimizerMode::On] {
            for rename in [false, true] {
                for style in [Style::Readable, Style::Compact] {
                    for target in [EcmaTarget::Es2018, EcmaTarget::Latest] {
                        let original = fixture();
                        let declarations: Vec<_> = original
                            .iter()
                            .cloned()
                            .map(|decl| {
                                let decl = optimizer::analyze(decl, mode).decl;
                                let mut decl = if jir {
                                    sroa::analyze_jir(decl, mode)
                                } else {
                                    sroa::analyze(decl, mode)
                                }
                                .decl;
                                if mode != OptimizerMode::On {
                                    assert!(original.contains(&decl));
                                }
                                if rename {
                                    minify::shorten_locals_for_measurement(&mut decl);
                                }
                                decl
                            })
                            .collect();
                        let text =
                            js::program_to_string_styled_for_target(&declarations, style, target);
                        let mut child = Command::new("node")
                            .arg("-")
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .spawn()
                            .expect("Node is required for the exception regression");
                        child
                            .stdin
                            .take()
                            .unwrap()
                            .write_all(format!("{text}\n{harness}").as_bytes())
                            .unwrap();
                        let output = child.wait_with_output().unwrap();
                        assert!(
                            output.status.success(),
                            "jir={jir} {:?} {rename} {style:?} {target:?}\n{}\n{text}",
                            mode,
                            String::from_utf8_lossy(&output.stderr)
                        );
                        assert_eq!(output.stdout, b"80\n");
                    }
                }
            }
        }
    }
}

#[test]
fn catch_visitors_and_maps_include_both_scopes_and_the_binding() {
    let loc = |line| Loc {
        file: "cleanup.rs".to_owned(),
        line,
        col: 0,
    };
    let stmt = catching(
        vec![
            Stmt::Loc(loc(10)),
            js::expr_stmt(invoke("hostCall", vec![])),
        ],
        "caught",
        vec![
            Stmt::Loc(loc(20)),
            js::expr_stmt(invoke("cleanupCall", vec![])),
            js::throw(js::id("caught")),
        ],
    );
    let mut refs = BTreeSet::new();
    js::visit_idents(&stmt, &mut |name| {
        refs.insert(name.to_owned());
    });
    assert_eq!(
        refs,
        BTreeSet::from([
            "hostCall".to_owned(),
            "cleanupCall".to_owned(),
            "caught".to_owned()
        ])
    );
    let mut bindings = Vec::new();
    js::visit_bindings(&stmt, &mut |name| bindings.push(name.to_owned()));
    assert_eq!(bindings, ["caught"]);
    for style in [Style::Readable, Style::Compact] {
        let (text, mappings) = js::program_to_string_mapped_styled(
            std::slice::from_ref(&stmt),
            BTreeMap::from([("caught".to_owned(), "exception".to_owned())]),
            Some(loc(1)),
            style,
        );
        for (name, line) in [("hostCall", 10), ("cleanupCall", 20)] {
            let start = text.find(name).unwrap();
            let generated_line = text[..start].bytes().filter(|b| *b == b'\n').count() as u32;
            let generated_col = text[..start]
                .rsplit('\n')
                .next()
                .unwrap()
                .encode_utf16()
                .count() as u32;
            let mapping = mappings
                .iter()
                .rev()
                .find(|m| (m.gen_line, m.gen_col) <= (generated_line, generated_col))
                .unwrap();
            assert_eq!(mapping.loc.line, line, "{style:?}: {text}");
        }
        assert!(mappings
            .iter()
            .any(|mapping| mapping.name.as_deref() == Some("exception")));
    }
}
