use super::*;
use std::io::Write;
use std::process::{Command, Stdio};

const RUNTIME: &str = include_str!("../../../../../runtime/shim.js");

fn call(interns: &mut Interns, name: &str, args: Vec<ValueId>) -> OpKind {
    OpKind::Call {
        callee: Callee::Direct {
            item: item(2),
            link_name: interns.intern_str(name),
        },
        function: None,
        args,
        track_caller: None,
        semantic: CallSemantic::Ordinary,
        host: None,
    }
}

fn fixture(interns: &mut Interns) -> Func {
    let mut b = FuncBuilder::new(item(1), signature(vec![I32], I32));
    let x = b.param(I32, NO_ORIGIN);
    let entry = FuncBuilder::ENTRY;
    let first = b.block();
    let second = b.block();
    let cleanup = b.block();
    let cleaned = b.block();
    let outer = b.block();
    let state = b.cell(I32, false, NO_ORIGIN);
    let carry = b.block_param(first, I32, NO_ORIGIN);
    let phase = b.block_param(cleanup, I32, NO_ORIGIN);
    let zero = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 0, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let one = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 1, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    let two = b.op(
        entry,
        OpKind::Const(ConstTree::Scalar { bits: 2, bytes: 4 }),
        I32,
        NO_ORIGIN,
    );
    b.effect(entry, OpKind::CellAlloc(state), NO_ORIGIN);
    b.effect(
        entry,
        OpKind::CellStore {
            cell: state,
            v: zero,
        },
        NO_ORIGIN,
    );
    let before = b.op(entry, call(interns, "before", vec![x]), I32, NO_ORIGIN);
    let a = b.op(
        entry,
        call(interns, "$invoke0", vec![one, before]),
        I32,
        NO_ORIGIN,
    );
    b.terminate(
        entry,
        RTerm::Invoke {
            normal: (first, vec![before]),
            unwind: (cleanup, vec![one]),
        },
    );
    b.effect(first, OpKind::CellStore { cell: state, v: a }, NO_ORIGIN);
    let result = b.op(
        first,
        call(interns, "$invoke0", vec![two, carry]),
        I32,
        NO_ORIGIN,
    );
    b.terminate(
        first,
        RTerm::Invoke {
            normal: (second, vec![]),
            unwind: (cleanup, vec![two]),
        },
    );
    b.terminate(second, RTerm::Return(Some(result)));
    let loaded = b.op(cleanup, OpKind::CellLoad(state), I32, NO_ORIGIN);
    b.effect(
        cleanup,
        call(interns, "$exception", vec![phase, loaded]),
        NO_ORIGIN,
    );
    b.terminate(
        cleanup,
        RTerm::Invoke {
            normal: (cleaned, vec![]),
            unwind: (outer, vec![]),
        },
    );
    b.terminate(cleaned, RTerm::Goto(outer, vec![]));
    b.effect(outer, call(interns, "$caught", vec![]), NO_ORIGIN);
    b.terminate(outer, RTerm::Throw(None));
    b.finish()
}

#[test]
fn invoke_result_and_rethrow_require_their_control_path() {
    let mut interns = world();
    let func = fixture(&mut interns);
    func.verify(&interns.types).unwrap();
    let result = func.blocks[0].ops.last().unwrap().result.unwrap();
    let mut bad = func.clone();
    bad.blocks[3].term = RTerm::Return(Some(result));
    assert!(bad
        .verify(&interns.types)
        .unwrap_err()
        .to_string()
        .contains("not dominated"));
    let mut bad = func.clone();
    bad.blocks[2].term = RTerm::Goto(RBlockId(1), vec![func.params[0]]);
    assert!(bad
        .verify(&interns.types)
        .unwrap_err()
        .to_string()
        .contains("must be private"));
    let mut bad = func.clone();
    bad.blocks[2].term = RTerm::Goto(RBlockId(5), vec![]);
    assert!(bad
        .verify(&interns.types)
        .unwrap_err()
        .to_string()
        .contains("no pending exception"));
    let mut bad = func.clone();
    bad.blocks[0].ops.last_mut().unwrap().kind = OpKind::Copy(func.params[0]);
    assert!(bad
        .verify(&interns.types)
        .unwrap_err()
        .to_string()
        .contains("call or drop"));
    let mut bad = func.clone();
    if let RTerm::Invoke { normal, .. } = &mut bad.blocks[0].term {
        normal.1[0] = result;
    }
    assert!(bad
        .verify(&interns.types)
        .unwrap_err()
        .to_string()
        .contains("not dominated"));
    let mut bad = func.clone();
    if let RTerm::Invoke { unwind, .. } = &mut bad.blocks[0].term {
        unwind.1.clear();
    }
    assert!(bad
        .verify(&interns.types)
        .unwrap_err()
        .to_string()
        .contains("arguments"));
    let bytes = postcard::to_allocvec(&func).unwrap();
    assert_eq!(postcard::from_bytes::<Func>(&bytes).unwrap(), func);
}

#[test]
fn invoke_emission_preserves_order_state_and_exception_identity() {
    let mut interns = world();
    let func = fixture(&mut interns);
    func.verify(&interns.types).unwrap();
    let body = body_of(BodyKind::Function, func.sig.clone());
    let harness = r#"
const assert = require('node:assert/strict');
let trace, failAt, cleanupThrows, outerThrows, thrown, replacement;
let abort;
try { __rt.js_abort('panic'); } catch (error) { abort = error; }
const trap = __rt.abort_error(new Error('rustc_codegen_js: entered unreachable code'));
const revoked = Proxy.revocable({}, {}); revoked.revoke();
const inputs = [undefined, null, false, 0, -0, NaN, 'error', Symbol('error'), {},
  new Error('rust abort: panic'), revoked.proxy, abort, trap];
function before(x) { trace.push('before'); if (failAt === 0) throw thrown; return x; }
function $invoke0(n, x) { trace.push(n); if (failAt === n) throw thrown; return x + n; }
function $exception(phase, state) {
  trace.push(['cleanup', phase, state]); if (cleanupThrows) throw replacement;
}
function $caught() { trace.push('outer'); if (outerThrows) throw replacement; }
let checks = 0;
for (thrown of inputs) for (replacement of [{}, abort]) {
  for (failAt of [-1, 0, 1, 2]) {
    for (cleanupThrows of [false, true]) for (outerThrows of [false, true]) {
      trace = [];
      let caught = false, value;
      try { value = entry(40); } catch (error) { caught = true; value = error; }
      assert.equal(caught, failAt !== -1);
      const aborts = thrown === abort || thrown === trap;
      const cleans = failAt > 0 && !aborts;
      const outer = cleans && !(cleanupThrows && replacement === abort);
      assert.ok(Object.is(value, failAt === -1 ? 42 :
        cleans && (cleanupThrows || outerThrows) ? replacement : thrown));
      const expected = ['before'];
      if (failAt !== 0) expected.push(1);
      if (failAt === -1 || failAt === 2) expected.push(2);
      if (cleans) expected.push(['cleanup', failAt, failAt === 1 ? 0 : 41]);
      if (outer) expected.push('outer');
      assert.deepEqual(trace, expected);
      checks++;
    }
  }
}
console.log(checks);
"#;
    check_emission(&interns, &body, &func, harness, 416, "invoke");
}

fn check_emission(
    interns: &Interns,
    body: &Body,
    func: &Func,
    harness: &str,
    cases: usize,
    label: &str,
) {
    use rcgjs_object::opts::{EcmaTarget, OptimizerMode};
    use rcgjs_object::{minify, optimizer, sroa};
    let summary = summary(&[]);
    let name = JsName::new("entry".to_owned());
    for dispatch in [false, true] {
        for mode in [OptimizerMode::Off, OptimizerMode::Report, OptimizerMode::On] {
            for rename in [false, true] {
                for style in [jsast::Style::Readable, jsast::Style::Compact] {
                    let opts = Opts {
                        optimizer: mode,
                        ..Opts::default()
                    };
                    let mut optimized = func.clone();
                    crate::opt::optimize(
                        &mut optimized,
                        &interns.types,
                        interns,
                        &opts,
                        &mut crate::opt::OptStats::default(),
                    )
                    .unwrap();
                    optimized
                        .verify_carriers(&interns.types, crate::opt::observed_carriers(&optimized))
                        .unwrap();
                    let func = &optimized;
                    let plan = crate::lower::fuse::plan(func, None, &interns.types, interns, &opts);
                    assert!(plan
                        .inline
                        .iter()
                        .all(|value| func.blocks.iter().any(|block| block.ops.iter().any(
                            |op| op.result == Some(*value)
                                && matches!(
                                    op.kind,
                                    OpKind::BorrowView { .. } | OpKind::RawView { .. }
                                )
                        ))));
                    for name in plan
                        .invoke_names
                        .values()
                        .chain(plan.exception_name.iter())
                        .chain(plan.catch_name.iter())
                    {
                        assert!(!interns.strings.contains(name));
                    }
                    let structured =
                        crate::lower::structure::structure(func, &plan, &interns.types, dispatch)
                            .unwrap();
                    let input = LowerInput {
                        name: &name,
                        summary: &summary,
                        body,
                        origins: None,
                        interns,
                        table: &interns.types,
                        opts: &opts,
                        items: super::super::host::EMPTY_ITEMS,
                        drop_glue: super::super::host::EMPTY_DROP_GLUE,
                        vtables: &super::super::unsize::EMPTY_VTABLES,
                        spills: &[],
                        abi: None,
                        physical: None,
                        call_retargets: crate::lower::EMPTY_CALL_RETARGETS,
                        bigints: None,
                    };
                    let lowered = function::lower(&input, func, &plan, structured.region).unwrap();
                    let mut decl =
                        sroa::analyze_jir(optimizer::analyze(lowered.decl, mode).decl, mode).decl;
                    if rename {
                        minify::shorten_locals_for_measurement(&mut decl);
                    }
                    let source =
                        jsast::stmt_to_string_styled_for_target(&decl, style, EcmaTarget::Es2018);
                    if let Some(dir) = std::env::var_os("RCGJS_INVOKE_CAPTURE") {
                        let dir = std::path::PathBuf::from(dir);
                        std::fs::create_dir_all(&dir).unwrap();
                        let path =
                            dir.join(format!("{label}-{dispatch}-{mode:?}-{rename}-{style:?}.js"));
                        std::fs::write(path, format!("{RUNTIME}\n{source}\n{harness}")).unwrap();
                    }
                    let mut child = Command::new("node")
                        .arg("-")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .expect("Node is required");
                    write!(
                        child.stdin.take().unwrap(),
                        "{RUNTIME}\n{source}\n{harness}"
                    )
                    .unwrap();
                    let output = child.wait_with_output().unwrap();
                    assert!(output.status.success(), "dispatch={dispatch}, mode={mode:?}, rename={rename}, style={style:?}\n{source}\n{}", String::from_utf8_lossy(&output.stderr));
                    assert_eq!(
                        String::from_utf8_lossy(&output.stdout).trim(),
                        cases.to_string()
                    );
                }
            }
        }
    }
}

#[test]
fn native_string_deref_follows_reborrows_across_exception_boundaries() {
    use crate::analysis::valueflow::harness::{base_interns, REF_STRING, STRING};
    use crate::jir::repr::Carrier;
    use crate::jir::semantic::StandardOperation;
    for carrier in [Carrier::HostString, Carrier::StringBuilder] {
        for depth in [0, 1, 12] {
            let mut interns = base_interns();
            let ordinary = world();
            for ty in [STR, STR_REF] {
                interns
                    .types
                    .insert(ordinary.types.get(ty).unwrap().clone());
            }
            let mut b = FuncBuilder::new(item(1), signature(vec![STRING], STR_REF));
            let value = b.param(STRING, NO_ORIGIN);
            let entry = FuncBuilder::ENTRY;
            let normal = b.block();
            let cleanup = b.block();
            let mut reference = b.op(
                entry,
                OpKind::BorrowView {
                    root: Root::Value(value),
                    path: vec![],
                    mutable: false,
                },
                REF_STRING,
                NO_ORIGIN,
            );
            for _ in 0..depth {
                reference = b.op(entry, OpKind::Copy(reference), REF_STRING, NO_ORIGIN);
                reference = b.op(entry, OpKind::Move(reference), REF_STRING, NO_ORIGIN);
                reference = b.op(
                    entry,
                    OpKind::BorrowView {
                        root: Root::Value(reference),
                        path: vec![Proj::Deref],
                        mutable: false,
                    },
                    REF_STRING,
                    NO_ORIGIN,
                );
            }
            let mut operation = call(&mut interns, "baseline_string", vec![reference]);
            let OpKind::Call { semantic, .. } = &mut operation else {
                unreachable!()
            };
            *semantic = CallSemantic::Standard(StandardOperation::StringAsStr);
            let result = b.op(entry, operation, STR_REF, NO_ORIGIN);
            b.terminate(
                entry,
                RTerm::Invoke {
                    normal: (normal, vec![]),
                    unwind: (cleanup, vec![]),
                },
            );
            b.terminate(normal, RTerm::Return(Some(result)));
            b.terminate(cleanup, RTerm::Throw(None));
            let mut func = b.finish();
            func.values[value.0 as usize].carrier = carrier;
            let body = body_of(BodyKind::Function, func.sig.clone());
            let input = if carrier == Carrier::HostString {
                "text"
            } else {
                "{text,cap:32}"
            };
            let harness = format!(
                r#"
function baseline_string() {{ throw new Error('native value reached baseline String::deref'); }}
for (const text of ['', 'plain', '\u00e9', '\ud83d\ude03']) {{
  if (entry({input}) !== text) throw new Error('native string reborrow changed its value');
}}
console.log(4);
"#
            );
            check_emission(
                &interns,
                &body,
                &func,
                &harness,
                4,
                &format!("native-reborrow-{carrier:?}-{depth}"),
            );
        }
    }
}

#[test]
fn protected_drop_runs_each_loop_iteration_and_rethrows_its_error() {
    let mut interns = world();
    let mut b = FuncBuilder::new(item(1), signature(vec![], I32));
    let entry = FuncBuilder::ENTRY;
    let header = b.block();
    let normal = b.block();
    let done = b.block();
    let cleanup = b.block();
    let index = b.block_param(header, I32, NO_ORIGIN);
    let state = b.cell(I32, true, NO_ORIGIN);
    b.effect(entry, OpKind::CellAlloc(state), NO_ORIGIN);
    let mut constant = |bits| {
        b.op(
            entry,
            OpKind::Const(ConstTree::Scalar { bits, bytes: 4 }),
            I32,
            NO_ORIGIN,
        )
    };
    let zero = constant(0);
    let one = constant(1);
    let three = constant(3);
    b.terminate(entry, RTerm::Goto(header, vec![zero]));
    let next = b.op(
        header,
        OpKind::Binary {
            op: BinaryOp::Add,
            l: index,
            r: one,
            ty: I32,
        },
        I32,
        NO_ORIGIN,
    );
    b.effect(
        header,
        OpKind::CellStore {
            cell: state,
            v: next,
        },
        NO_ORIGIN,
    );
    b.effect(
        header,
        OpKind::Drop {
            root: Root::Cell(state),
            path: vec![],
            glue: DropGlue::Direct {
                item: item(2),
                link_name: interns.intern_str("drop_value"),
            },
        },
        NO_ORIGIN,
    );
    b.terminate(
        header,
        RTerm::Invoke {
            normal: (normal, vec![]),
            unwind: (cleanup, vec![]),
        },
    );
    let more = b.op(
        normal,
        OpKind::Binary {
            op: BinaryOp::Lt,
            l: next,
            r: three,
            ty: I32,
        },
        BOOL,
        NO_ORIGIN,
    );
    b.terminate(
        normal,
        RTerm::Switch {
            scrut: more,
            scrut_ty: BOOL,
            cases: vec![(1, header, vec![next])],
            default: (done, vec![]),
        },
    );
    b.terminate(done, RTerm::Return(Some(next)));
    b.effect(cleanup, call(&mut interns, "outer", vec![]), NO_ORIGIN);
    b.terminate(cleanup, RTerm::Throw(None));
    let func = b.finish();
    func.verify(&interns.types).unwrap();
    let body = body_of(BodyKind::Function, func.sig.clone());
    let harness = r#"
const assert = require('node:assert/strict');
let trace, failAt, thrown;
function drop_value(slot) {
  const n = slot.buf[slot.off]; trace.push(n); if (n === failAt) throw thrown;
}
function outer() { trace.push('outer'); }
for (thrown of [undefined, Symbol('drop')]) for (failAt of [0, 1, 2, 3]) {
  trace = []; let caught = false, result;
  try { result = entry(); } catch (error) { result = error; caught = true; }
  assert.equal(caught, failAt !== 0);
  assert.ok(Object.is(result, failAt === 0 ? 3 : thrown));
  assert.deepEqual(trace, failAt === 0 ? [1, 2, 3] :
    Array.from({length: failAt}, (_, i) => i + 1).concat('outer'));
}
"#;
    for dispatch in [false, true] {
        let plan =
            crate::lower::fuse::plan(&func, None, &interns.types, &interns, &Opts::default());
        let structured =
            crate::lower::structure::structure(&func, &plan, &interns.types, dispatch).unwrap();
        let source = lower(
            &interns,
            "entry",
            &["drop_value", "outer"],
            &body,
            &func,
            &plan,
            structured.region,
        );
        let mut child = Command::new("node")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Node is required");
        write!(
            child.stdin.take().unwrap(),
            "{RUNTIME}\n{source}\n{harness}"
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{source}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn exception_optimization_preserves_report_mode_and_interpretation_declines() {
    use rcgjs_object::jir::interp::{run_func, Hooks, Trap, Value};
    use rcgjs_object::opts::OptimizerMode;
    let mut interns = world();
    let func = fixture(&mut interns);
    let hooks = Hooks::new(&|_, _| panic!("unsupported function executed a hook"));
    let outcome = run_func(
        &func,
        &interns.types,
        &interns,
        vec![Value::int(40, 32, true)],
        &hooks,
    );
    assert_eq!(
        outcome.ret,
        Err(Trap::Unsupported("exceptional call execution"))
    );
    assert!(outcome.trace.is_empty());
    for optimizer in [OptimizerMode::Off, OptimizerMode::Report, OptimizerMode::On] {
        let opts = Opts {
            optimizer,
            ..Opts::default()
        };
        let mut candidate = func.clone();
        let result = crate::opt::optimize(
            &mut candidate,
            &interns.types,
            &interns,
            &opts,
            &mut crate::opt::OptStats::default(),
        );
        result.unwrap();
        candidate.verify(&interns.types).unwrap();
        if optimizer != OptimizerMode::On {
            assert_eq!(candidate, func);
        }
    }
}

/// Repeated calls replace a destination shared with the cleanup path. Its old value must
/// survive a failed call in SSA, in a borrowed cell, and through a projected reference.
fn semantic_fixture(interns: &mut Interns, shape: usize) -> Body {
    use rcgjs_object::jir::semantic::{
        BasicBlock, BlockId, Call, Constant, Local, LocalId, LocalRole, Operand, OperandKind,
        Place, Projection, Rvalue, Statement, StatementKind, Terminator, Unwind,
    };
    let place = |local, ty| Place {
        local: LocalId(local),
        projection: vec![],
        ty,
    };
    let read = |place: Place| Operand {
        ty: place.ty,
        kind: OperandKind::Copy(place),
    };
    let scalar = |bits| Operand {
        ty: I32,
        kind: OperandKind::Constant(Constant::Scalar { bits, bytes: 4 }),
    };
    let assign = |destination, value| Statement {
        kind: StatementKind::Assign { destination, value },
    };
    let dest = if shape == 2 {
        Place {
            local: LocalId(2),
            projection: vec![Projection::Deref],
            ty: I32,
        }
    } else if shape == 4 {
        Place {
            local: LocalId(3),
            projection: vec![Projection::Field { field: 0, ty: I32 }],
            ty: I32,
        }
    } else {
        place(3, I32)
    };
    let mut initial = vec![
        assign(dest.clone(), Rvalue::Use(scalar(7))),
        assign(place(4, I32), Rvalue::Use(scalar(0))),
    ];
    if shape == 4 {
        initial[0] = assign(
            place(3, POINT),
            Rvalue::Aggregate {
                result: POINT,
                kind: rcgjs_object::jir::semantic::AggregateKind::Adt {
                    definition: def(11),
                    variant: 0,
                    active_field: None,
                },
                operands: vec![scalar(7), scalar(99)],
            },
        );
    }
    if shape == 1 {
        initial.push(assign(
            place(6, REF_I32),
            Rvalue::Reference {
                kind: rcgjs_object::jir::semantic::BorrowKind::Shared,
                place: dest.clone(),
            },
        ));
    }
    let mut call = |name: &str, args, destination, target, unwind| {
        Terminator::Call(Call {
            function: Operand {
                ty: I32,
                kind: OperandKind::Constant(Constant::ZeroSized),
            },
            callee: Callee::Direct {
                item: item(2),
                link_name: interns.intern_str(name),
            },
            semantic: CallSemantic::Ordinary,
            arguments: args,
            destination,
            target,
            unwind,
            track_caller: None,
            host: None,
        })
    };
    let step = call(
        "step",
        vec![read(place(4, I32)), read(dest.clone())],
        dest.clone(),
        Some(BlockId(1)),
        Unwind::Cleanup(BlockId(4)),
    );
    let cleanup = call(
        "cleanup",
        vec![read(place(4, I32)), read(dest.clone())],
        place(7, UNIT),
        Some(BlockId(5)),
        Unwind::Cleanup(BlockId(6)),
    );
    let outer = call(
        "outer",
        vec![],
        place(7, UNIT),
        Some(BlockId(7)),
        Unwind::Continue,
    );
    let cleanup = if shape == 3 {
        Terminator::Drop {
            place: dest.clone(),
            target: BlockId(5),
            unwind: Unwind::Cleanup(BlockId(6)),
            replace: false,
            async_drop: None,
            glue: DropGlue::Direct {
                item: item(2),
                link_name: interns.intern_str("drop_value"),
            },
        }
    } else {
        cleanup
    };
    let data = vec![
        (
            initial,
            Terminator::Switch {
                discriminant: read(place(1, BOOL)),
                targets: vec![(0, BlockId(1))],
                otherwise: BlockId(3),
            },
        ),
        (
            vec![assign(
                place(5, BOOL),
                Rvalue::Binary {
                    op: BinaryOp::Lt,
                    left: read(place(4, I32)),
                    right: scalar(3),
                },
            )],
            Terminator::Switch {
                discriminant: read(place(5, BOOL)),
                targets: vec![(0, BlockId(3))],
                otherwise: BlockId(2),
            },
        ),
        (
            vec![assign(
                place(4, I32),
                Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: read(place(4, I32)),
                    right: scalar(1),
                },
            )],
            step,
        ),
        (
            vec![assign(place(0, I32), Rvalue::Use(read(dest)))],
            Terminator::Return,
        ),
        (vec![], cleanup),
        (vec![], Terminator::Goto(BlockId(6))),
        (vec![], outer),
        (vec![], Terminator::UnwindResume),
    ];
    Body {
        item: item(1),
        kind: BodyKind::Function,
        semantic: BodySemantic::Ordinary,
        signature: signature(vec![BOOL, REF_I32], I32),
        locals: [
            I32,
            BOOL,
            REF_I32,
            if shape == 4 { POINT } else { I32 },
            I32,
            BOOL,
            REF_I32,
            UNIT,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, ty)| Local {
            id: LocalId(index as u32),
            ty,
            role: match index {
                0 => LocalRole::Return,
                1 | 2 => LocalRole::Argument,
                _ => LocalRole::Temporary,
            },
        })
        .collect(),
        blocks: data
            .into_iter()
            .enumerate()
            .map(|(index, (statements, terminator))| BasicBlock {
                id: BlockId(index as u32),
                cleanup: index >= 4,
                statements,
                terminator,
            })
            .collect(),
    }
}

#[test]
fn semantic_cleanup_preserves_destination_state_across_loop_and_shared_targets() {
    let harness = r#"
const assert = require('node:assert/strict');
let abort; try { __rt.js_abort('abort'); } catch (error) { abort = error; }
const revoked = Proxy.revocable({}, {}); revoked.revoke();
const errors = [undefined, null, false, 0, -0, NaN, 'error', Symbol('error'), {},
  new Error('rust abort: abort'), revoked.proxy, abort];
let trace, thrown, failAt, cleanupThrows, outerThrows, replacement;
function step(index, value) {
  trace.push(['step', index, value]); if (index === failAt) throw thrown; return value + index;
}
function cleanup(index, value) { trace.push(['cleanup', index, value]); if (cleanupThrows) throw replacement; }
function drop_value(slot) { cleanup(failAt, slot.buf[slot.off]); }
function outer() { trace.push('outer'); if (outerThrows) throw replacement; }
let checks = 0;
for (const skip of [false, true]) for (thrown of errors) for (replacement of [{}, abort]) {
  for (failAt of [0, 1, 2, 3]) for (cleanupThrows of [false, true]) for (outerThrows of [false, true]) {
    trace = []; let value, caught = false;
    try { value = entry(skip, {buf: [7], off: 0}); } catch (error) { value = error; caught = true; }
    const fails = !skip && failAt !== 0;
    assert.equal(caught, fails);
    assert.ok(Object.is(value, !fails ? skip ? 7 : 13 : thrown === abort ? abort :
      cleanupThrows || outerThrows ? replacement : thrown));
    const expected = []; let old = 7;
    if (!skip) for (let n = 1; n <= (fails ? failAt : 3); n++) {
      expected.push(['step', n, old]); if (!fails || n !== failAt) old += n;
    }
    if (fails && thrown !== abort) {
      expected.push(['cleanup', failAt, old]);
      if (!(cleanupThrows && replacement === abort)) expected.push('outer');
    }
    assert.deepEqual(trace, expected); checks++;
  }
}
console.log(checks);
"#;
    for shape in 0..5 {
        let mut interns = world();
        let body = semantic_fixture(&mut interns, shape);
        body.verify(&interns.types, &interns).unwrap();
        let func = crate::lower::ssa::build(&body, None, &interns.types, &interns).unwrap();
        func.verify(&interns.types).unwrap();
        assert_eq!(func.blocks.len(), body.blocks.len() + 2);
        assert!(matches!(func.blocks[2].term, RTerm::Invoke { .. }));
        assert!(matches!(func.blocks[4].term, RTerm::Invoke { .. }));
        let RTerm::Invoke { normal, .. } = &func.blocks[2].term else {
            unreachable!()
        };
        assert_ne!(normal.0, RBlockId(1));
        assert!(matches!(
            func.blocks[normal.0 .0 as usize].term,
            RTerm::Goto(RBlockId(1), _)
        ));
        check_emission(
            &interns,
            &body,
            &func,
            harness,
            768,
            &format!("semantic-{shape}"),
        );
    }
}

#[test]
fn semantic_cleanup_obeys_initialization_flags_and_zero_sized_call_results() {
    use rcgjs_object::jir::semantic::{
        BasicBlock, BlockId, Constant, LocalId, Operand, OperandKind, Place, Rvalue, Statement,
        StatementKind, Terminator, Unwind,
    };
    let mut interns = world();
    let mut body = semantic_fixture(&mut interns, 3);
    let place = |local, ty| Place {
        local: LocalId(local),
        projection: vec![],
        ty,
    };
    let set = |local, ty, bits, bytes| Statement {
        kind: StatementKind::Assign {
            destination: place(local, ty),
            value: Rvalue::Use(Operand {
                ty,
                kind: OperandKind::Constant(Constant::Scalar { bits, bytes }),
            }),
        },
    };
    let Terminator::Call(mut make) = body.blocks[2].terminator.clone() else {
        unreachable!()
    };
    make.callee = Callee::Direct {
        item: item(2),
        link_name: interns.intern_str("make"),
    };
    make.arguments.clear();
    make.target = Some(BlockId(1));
    make.unwind = Unwind::Cleanup(BlockId(3));
    let Terminator::Call(mut step) = body.blocks[2].terminator.clone() else {
        unreachable!()
    };
    step.arguments.clear();
    step.destination = place(7, UNIT);
    step.target = Some(BlockId(2));
    step.unwind = Unwind::Cleanup(BlockId(3));
    let drop = |target| Terminator::Drop {
        place: place(3, I32),
        target: BlockId(target),
        unwind: Unwind::Continue,
        replace: false,
        async_drop: None,
        glue: DropGlue::Direct {
            item: item(2),
            link_name: match &body.blocks[4].terminator {
                Terminator::Drop {
                    glue: DropGlue::Direct { link_name, .. },
                    ..
                } => *link_name,
                _ => unreachable!(),
            },
        },
    };
    let data = vec![
        (
            vec![set(0, I32, 0, 4), set(5, BOOL, 0, 1)],
            Terminator::Call(make),
        ),
        (vec![set(5, BOOL, 1, 1)], Terminator::Call(step)),
        (vec![], drop(6)),
        (
            vec![],
            Terminator::Switch {
                discriminant: Operand {
                    ty: BOOL,
                    kind: OperandKind::Copy(place(5, BOOL)),
                },
                targets: vec![(0, BlockId(5))],
                otherwise: BlockId(4),
            },
        ),
        (vec![], drop(5)),
        (vec![], Terminator::UnwindResume),
        (vec![], Terminator::Return),
    ];
    body.blocks = data
        .into_iter()
        .enumerate()
        .map(|(index, (statements, terminator))| BasicBlock {
            id: BlockId(index as u32),
            cleanup: (3..6).contains(&index),
            statements,
            terminator,
        })
        .collect();
    body.verify(&interns.types, &interns).unwrap();
    let func = crate::lower::ssa::build(&body, None, &interns.types, &interns).unwrap();
    let harness = r#"
const assert = require('node:assert/strict');
let abort; try { __rt.js_abort('abort'); } catch (e) { abort = e; }
let trace, failAt, thrown, replacement, dropThrows;
function make() { trace.push('make'); if (failAt === 1) throw thrown; return 41; }
function step() { trace.push('step'); if (failAt === 2) throw thrown; }
function drop_value(slot) { assert.equal(slot.buf[slot.off], 41); trace.push('drop'); if (dropThrows) throw replacement; }
let checks = 0;
for (thrown of [undefined, null, {}, Symbol('error'), new Error('error'), abort]) {
  for (replacement of [{}, abort]) for (failAt of [0, 1, 2]) for (dropThrows of [false, true]) {
    trace = []; let caught = false, value;
    try { value = entry(false, {buf: [0], off: 0}); } catch (error) { caught = true; value = error; }
    const drops = failAt !== 1 && !(failAt === 2 && thrown === abort);
    assert.equal(caught, failAt !== 0 || dropThrows);
    assert.ok(Object.is(value, drops && dropThrows ? replacement : failAt === 0 ? 0 : thrown));
    assert.deepEqual(trace, failAt === 1 ? ['make'] : drops ? ['make', 'step', 'drop'] : ['make', 'step']);
    checks++;
  }
}
console.log(checks);
"#;
    check_emission(&interns, &body, &func, harness, 72, "initialization");
}

#[test]
fn semantic_zero_sized_destination_keeps_its_synthetic_place() {
    use rcgjs_object::jir::semantic::{
        BasicBlock, BlockId, Constant, Local, LocalId, LocalRole, Operand, OperandKind, Place,
        Projection, Terminator, Unwind,
    };
    use rcgjs_object::jir::semantic::{Rvalue, Statement, StatementKind};
    let mut interns = world();
    let mut point = interns.types.get(POINT).unwrap().clone();
    let TypeKind::Adt { variants, .. } = &mut point.kind else {
        unreachable!()
    };
    variants[0].fields[0].ty = UNIT;
    point.js.fields[0].ty = UNIT;
    point.layout.size = Some(4);
    interns.types.replace(point);
    let mut body = semantic_fixture(&mut interns, 0);
    let Terminator::Call(mut step) = body.blocks[2].terminator.clone() else {
        unreachable!()
    };
    step.arguments.clear();
    step.target = Some(BlockId(1));
    step.unwind = Unwind::Cleanup(BlockId(2));
    step.destination = Place {
        local: LocalId(1),
        projection: vec![Projection::Deref, Projection::Field { field: 0, ty: UNIT }],
        ty: UNIT,
    };
    let zero_destination = step.destination.clone();
    step.destination = Place {
        local: LocalId(3),
        projection: vec![],
        ty: I32,
    };
    let mut cleanup = step.clone();
    cleanup.callee = Callee::Direct {
        item: item(2),
        link_name: interns.intern_str("cleanup"),
    };
    cleanup.target = Some(BlockId(3));
    cleanup.unwind = Unwind::Continue;
    cleanup.destination = Place {
        local: LocalId(2),
        projection: vec![],
        ty: UNIT,
    };
    cleanup.function = Operand {
        ty: I32,
        kind: OperandKind::Constant(Constant::ZeroSized),
    };
    body.signature = signature(vec![REF_POINT], UNIT);
    body.locals = [UNIT, REF_POINT, UNIT, I32]
        .into_iter()
        .enumerate()
        .map(|(index, ty)| Local {
            id: LocalId(index as u32),
            ty,
            role: match index {
                0 => LocalRole::Return,
                1 => LocalRole::Argument,
                _ => LocalRole::Temporary,
            },
        })
        .collect();
    body.blocks = [
        Terminator::Call(step),
        Terminator::Return,
        Terminator::Call(cleanup),
        Terminator::UnwindResume,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, terminator)| BasicBlock {
        id: BlockId(index as u32),
        cleanup: index >= 2,
        statements: vec![],
        terminator,
    })
    .collect();
    body.blocks[1].statements.push(Statement {
        kind: StatementKind::Assign {
            destination: zero_destination,
            value: Rvalue::Use(Operand {
                ty: UNIT,
                kind: OperandKind::Constant(Constant::ZeroSized),
            }),
        },
    });
    body.verify(&interns.types, &interns).unwrap();
    let func = crate::lower::ssa::build(&body, None, &interns.types, &interns).unwrap();
    assert!(func
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .any(|op| matches!(op.kind, OpKind::BorrowView { .. })
            && op.result.is_some_and(|value| func.ty(value) == Some(UNIT))));
    let harness = r#"
const assert = require('node:assert/strict');
let trace, fail, thrown;
function step() { trace.push('step'); if (fail) throw thrown; return 41; }
function cleanup() { trace.push('cleanup'); }
let checks = 0;
for (thrown of [undefined, null, {}, Symbol('error'), new Error('error'), false]) for (fail of [false, true]) {
  trace = []; let value, caught = false;
  const point = {x: undefined, y: 41};
  try { value = entry(point); } catch (error) { value = error; caught = true; }
  assert.equal(caught, fail); assert.ok(Object.is(value, fail ? thrown : undefined));
  assert.deepEqual(trace, fail ? ['step', 'cleanup'] : ['step']);
  assert.deepEqual(point, {x: undefined, y: 41}); checks++;
}
console.log(checks);
"#;
    check_emission(&interns, &body, &func, harness, 12, "zero-sized-place");
}

#[test]
fn semantic_diverging_call_rethrows_or_aborts_after_an_unexpected_return() {
    use rcgjs_object::jir::semantic::Terminator;
    let mut interns = world();
    let mut body = semantic_fixture(&mut interns, 0);
    let Terminator::Call(call) = &mut body.blocks[2].terminator else {
        unreachable!()
    };
    call.target = None;
    body.verify(&interns.types, &interns).unwrap();
    let func = crate::lower::ssa::build(&body, None, &interns.types, &interns).unwrap();
    let harness = r#"
const assert = require('node:assert/strict');
let abort; try { __rt.js_abort('abort'); } catch (e) { abort = e; }
let trace, fail, thrown;
function step(index, value) { trace.push(['step', index, value]); if (fail) throw thrown; return 42; }
function cleanup(index, value) { trace.push(['cleanup', index, value]); }
function outer() { trace.push('outer'); }
let checks = 0;
for (thrown of [undefined, null, {}, Symbol('error'), new Error('error'), abort]) for (fail of [false, true]) {
  trace = []; let value, caught = false;
  try { entry(false, {buf: [7], off: 0}); } catch (error) { value = error; caught = true; }
  assert.equal(caught, true);
  if (fail) assert.ok(Object.is(value, thrown));
  else { assert.equal(value.message, 'rustc_codegen_js: entered unreachable code'); assert.equal(__rt.is_abort(value), true); }
  assert.deepEqual(trace, fail && thrown !== abort ? [['step', 1, 7], ['cleanup', 1, 7], 'outer'] : [['step', 1, 7]]);
  checks++;
}
console.log(checks);
"#;
    check_emission(&interns, &body, &func, harness, 12, "diverging");
}

#[test]
fn synthetic_success_writes_retain_the_call_origin_and_termination_declines() {
    use rcgjs_object::jir::origin::{Origin, OriginId, OriginKind, OriginTable};
    use rcgjs_object::jir::semantic::{TerminateReason, Terminator};
    let mut interns = world();
    let mut body = semantic_fixture(&mut interns, 1);
    let origins = OriginTable {
        origins: body
            .blocks
            .iter()
            .map(|_| Origin {
                kind: OriginKind::Terminator,
                source: None,
                local_name: None,
                inline_stack: vec![],
            })
            .collect(),
        statements: vec![OriginId(0); body.blocks.iter().map(|block| block.statements.len()).sum()],
        terminators: (0..body.blocks.len())
            .map(|index| OriginId(index as u32))
            .collect(),
        locals: vec![OriginId(0); body.locals.len()],
        item_path: interns.intern_str("entry"),
        root: None,
    };
    let func = crate::lower::ssa::build(&body, Some(&origins), &interns.types, &interns).unwrap();
    let RTerm::Invoke { normal, .. } = func.blocks[2].term.clone() else {
        unreachable!()
    };
    let operations = &func.blocks[normal.0 .0 as usize].ops;
    assert!(operations
        .iter()
        .any(|op| matches!(op.kind, OpKind::CellStore { .. })));
    assert!(operations.iter().all(|op| op.origin == OriginId(2)));
    body.blocks[7].terminator = Terminator::UnwindTerminate(TerminateReason::InCleanup);
    assert!(
        crate::lower::ssa::build(&body, None, &interns.types, &interns)
            .unwrap_err()
            .to_string()
            .contains("separate abort boundary")
    );
}
