//! The dynamic host operations identified by `rcgjs.host.*` markers.
//! Both compiler lanes use this contract after decoding the marker identity.

use crate::evaluation::{captured, OperandEvaluation};
use crate::jsast::{self, BinOp, Expr, UnOp};
use crate::ptr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostOperation {
    Undefined,
    Null,
    IsUndefined,
    IsNull,
    Truthy,
    StrictEq,
    Add,
    TypeIs(&'static str),
    IsObject,
    IsInteger,
    BigIntRange,
    Erase,
    Invoke,
    Construct,
}

impl HostOperation {
    pub fn from_marker(marker: &str) -> Option<Self> {
        Some(match marker {
            "undefined" => Self::Undefined,
            "null" => Self::Null,
            "isundefined" => Self::IsUndefined,
            "isnull" => Self::IsNull,
            "truthy" => Self::Truthy,
            "stricteq" => Self::StrictEq,
            "add" => Self::Add,
            "isboolean" => Self::TypeIs("boolean"),
            "isnumber" => Self::TypeIs("number"),
            "isbigint" => Self::TypeIs("bigint"),
            "isstring" => Self::TypeIs("string"),
            "isfunction" => Self::TypeIs("function"),
            "isobject" => Self::IsObject,
            "isinteger" => Self::IsInteger,
            "bigintrange" => Self::BigIntRange,
            "erase" => Self::Erase,
            "invoke" => Self::Invoke,
            "construct" => Self::Construct,
            _ => return None,
        })
    }

    pub fn arity(self) -> usize {
        match self {
            Self::Undefined | Self::Null => 0,
            Self::StrictEq | Self::Add | Self::Construct => 2,
            Self::BigIntRange | Self::Invoke => 3,
            _ => 1,
        }
    }

    pub fn evaluation(self) -> OperandEvaluation {
        match self {
            Self::Undefined
            | Self::Null
            | Self::IsUndefined
            | Self::IsNull
            | Self::Truthy
            | Self::StrictEq
            | Self::TypeIs(_)
            | Self::Erase => OperandEvaluation::EagerOnce,
            // These spellings repeat or conditionally read an operand, or look up a
            // host function before consuming operands. Source arguments run first.
            _ => OperandEvaluation::Materialize,
        }
    }

    pub fn lower(self, arguments: Vec<Expr>) -> Result<Expr, String> {
        self.lower_with(arguments, false)
    }

    /// Calls with a proven private argument array need no Rust slice conversion.
    /// The JavaScript array remains an allocation charged to the caller.
    pub fn lower_array(self, arguments: Vec<Expr>) -> Result<Expr, String> {
        if !matches!(self, Self::Invoke | Self::Construct) {
            return Err("only invocation and construction accept an argument array".to_owned());
        }
        self.lower_with(arguments, true)
    }

    fn lower_with(self, arguments: Vec<Expr>, array: bool) -> Result<Expr, String> {
        if arguments.len() != self.arity() {
            return Err(format!(
                "host operation {self:?} takes {} arguments",
                self.arity()
            ));
        }
        if self.evaluation() == OperandEvaluation::Materialize
            && arguments.iter().any(|value| !captured(value))
        {
            let params = (0..arguments.len())
                .map(|index| format!("$host{index}"))
                .collect::<Vec<_>>();
            let values = params.iter().cloned().map(jsast::id).collect::<Vec<_>>();
            return Ok(jsast::call(
                jsast::arrow(params, self.emit(&values, array)),
                arguments,
            ));
        }
        Ok(self.emit(&arguments, array))
    }

    fn emit(self, values: &[Expr], array: bool) -> Expr {
        let one = || values[0].clone();
        match self {
            Self::Undefined => jsast::undefined(),
            Self::Null => jsast::null(),
            Self::IsUndefined => jsast::binary(BinOp::StrictEq, one(), jsast::undefined()),
            Self::IsNull => jsast::binary(BinOp::StrictEq, one(), jsast::null()),
            Self::Truthy => jsast::not(jsast::not(one())),
            Self::StrictEq => jsast::binary(BinOp::StrictEq, one(), values[1].clone()),
            Self::Add => jsast::rt_call("host_add", values.to_vec()),
            Self::TypeIs(name) => jsast::binary(
                BinOp::StrictEq,
                jsast::unary(UnOp::TypeOf, one()),
                jsast::string(name),
            ),
            Self::IsObject => jsast::binary(
                BinOp::And,
                jsast::binary(BinOp::StrictNe, one(), jsast::null()),
                jsast::binary(
                    BinOp::StrictEq,
                    jsast::unary(UnOp::TypeOf, one()),
                    jsast::string("object"),
                ),
            ),
            Self::IsInteger => jsast::method_call(jsast::id("Number"), "isInteger", vec![one()]),
            Self::BigIntRange => jsast::binary(
                BinOp::And,
                jsast::binary(BinOp::Ge, one(), values[1].clone()),
                jsast::binary(BinOp::Le, one(), values[2].clone()),
            ),
            Self::Erase => one(),
            Self::Invoke | Self::Construct => {
                let arguments = values.last().unwrap();
                let array = if array {
                    arguments.clone()
                } else {
                    jsast::method_call(
                        ptr::slot_buf(arguments.clone()),
                        "slice",
                        vec![
                            ptr::slot_off(arguments.clone()),
                            jsast::binary(
                                BinOp::Add,
                                ptr::slot_off(arguments.clone()),
                                ptr::slot_len(arguments.clone()),
                            ),
                        ],
                    )
                };
                let mut args = values[..values.len() - 1].to_vec();
                args.push(array);
                jsast::method_call(
                    jsast::id("Reflect"),
                    if self == Self::Invoke {
                        "apply"
                    } else {
                        "construct"
                    },
                    args,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expression(operation: HostOperation) -> String {
        let arguments = (0..operation.arity())
            .map(|index| jsast::call(jsast::id("read"), vec![jsast::num(index as f64)]))
            .collect();
        jsast::expr_to_string(&operation.lower(arguments).unwrap())
    }

    #[test]
    fn host_inputs_run_once_before_branching_and_host_lookup() {
        let source = format!(
            r#"'use strict';
const assert = require('node:assert/strict');
let values, trace, fail = -1;
const sentinel = Object.freeze({{}});
function read(index) {{ trace.push(index); if (index === fail) throw sentinel; return values[index]; }}
const Number = {{ get isInteger() {{ trace.push('integer'); return globalThis.Number.isInteger; }} }};
const Reflect = {{
  get apply() {{ trace.push('apply'); return globalThis.Reflect.apply; }},
  get construct() {{ trace.push('construct'); return globalThis.Reflect.construct; }}
}};
const object = () => {object};
for (const value of [null, undefined, false, 0, '', Symbol(), 1n, {{}}, [], () => 0]) {{
  values = [value]; trace = [];
  assert.equal(object(), value !== null && typeof value === 'object');
  assert.deepEqual(trace, [0]);
}}
values = [0n, 1n, 2n]; trace = [];
const range = () => {range};
assert.equal(range(), false);
assert.deepEqual(trace, [0, 1, 2]);
const integer = () => {integer};
values = [3.5]; trace = [];
assert.equal(integer(), false);
assert.deepEqual(trace, [0, 'integer']);
const invoke = () => {invoke};
const construct = () => {construct};
function target(...args) {{ return {{receiver: this, args}}; }}
Object.defineProperty(target, 'call', {{get() {{ throw sentinel; }} }});
const callable = new Proxy(target, {{apply(fn, receiver, args) {{
  trace.push('called'); return globalThis.Reflect.apply(fn, receiver, args);
}} }});
const pack = {{buf: ['skip', 1, sentinel, 'skip'], off: 1, len: 2}};
for (const receiver of [null, undefined, 4, 'text', sentinel]) {{
  values = [callable, receiver, pack]; trace = [];
  const result = invoke();
  assert.equal(result.receiver, receiver);
  assert.deepEqual(result.args, [1, sentinel]);
  assert.deepEqual(trace, [0, 1, 2, 'apply', 'called']);
}}
function Constructor(...args) {{ this.args = args; this.target = new.target; }}
values = [Constructor, pack]; trace = [];
const result = construct();
assert.ok(result instanceof Constructor);
assert.equal(result.target, Constructor);
assert.deepEqual(result.args, [1, sentinel]);
assert.deepEqual(trace, [0, 1, 'construct']);
for (const [run, inputs] of [
  [object, [{{}}]], [integer, [1]], [range, [0n, 1n, 2n]],
  [invoke, [callable, sentinel, pack]], [construct, [Constructor, pack]]
]) {{
  values = inputs;
  for (fail = 0; fail < inputs.length; fail++) {{
    trace = [];
    assert.throws(run, error => error === sentinel);
    assert.deepEqual(trace, Array.from({{length: fail + 1}}, (_, i) => i));
  }}
}}
fail = -1;
for (const reason of [null, undefined, false, 0, '', sentinel]) {{
  values = [function () {{ throw reason; }}, null, pack]; trace = [];
  assert.throws(invoke, error => error === reason);
  assert.deepEqual(trace, [0, 1, 2, 'apply']);
}}
values = [0, null, pack]; trace = [];
assert.throws(invoke, TypeError);
assert.deepEqual(trace, [0, 1, 2, 'apply']);
values = [() => 0, pack]; trace = [];
assert.throws(construct, TypeError);
assert.deepEqual(trace, [0, 1, 'construct']);
"#,
            object = expression(HostOperation::IsObject),
            range = expression(HostOperation::BigIntRange),
            integer = expression(HostOperation::IsInteger),
            invoke = expression(HostOperation::Invoke),
            construct = expression(HostOperation::Construct),
        );
        let output = std::process::Command::new("node")
            .args(["-e", &source])
            .output()
            .expect("Node is required by the host semantic tests");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn known_markers_validate_arity_and_unknown_markers_decline() {
        for marker in [
            "undefined",
            "null",
            "isundefined",
            "isnull",
            "truthy",
            "stricteq",
            "add",
            "isboolean",
            "isnumber",
            "isbigint",
            "isstring",
            "isfunction",
            "isobject",
            "isinteger",
            "bigintrange",
            "erase",
            "invoke",
            "construct",
        ] {
            let operation = HostOperation::from_marker(marker).unwrap();
            assert!(operation
                .lower(vec![jsast::null(); operation.arity()])
                .is_ok());
            assert!(operation
                .lower(vec![jsast::null(); operation.arity() + 1])
                .is_err());
        }
        assert_eq!(HostOperation::from_marker("unknown"), None);
    }
}
