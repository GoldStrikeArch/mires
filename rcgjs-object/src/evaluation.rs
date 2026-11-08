//! Operand evaluation obligations shared by expression fusion and intrinsic emission.

use crate::jir::semantic::BinaryOp;
use crate::jsast::{self, BinOp, Expr, Stmt};

/// Whether an emitter can consume expressions directly instead of captured operand values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperandEvaluation {
    /// Every operand runs once, unconditionally, in argument order, before the operation.
    /// Fusion still has to prove that moving it past intervening effects is legal.
    EagerOnce,
    /// Multiplicity, order, or conditional evaluation is not preserved by the spelling.
    /// Capture operands in source order before lowering. This is the default for unknown
    /// emitters, including operations that ignore an argument or use it only on a cold path.
    Materialize,
}

impl OperandEvaluation {
    /// Captures expressions before an emitter may repeat or conditionally consume them.
    /// Names must already denote captured values. Member/index reads are never exempt.
    pub fn capture(
        self,
        arguments: Vec<Expr>,
        mut bind: impl FnMut(Expr) -> (Stmt, Expr),
    ) -> (Vec<Stmt>, Vec<Expr>) {
        let mut setup = Vec::new();
        let arguments = arguments
            .into_iter()
            .map(|value| {
                if self == Self::EagerOnce || captured(&value) {
                    value
                } else {
                    let (statement, value) = bind(value);
                    setup.push(statement);
                    value
                }
            })
            .collect();
        (setup, arguments)
    }

    /// The evaluation contract of an intrinsic's JavaScript spelling.
    pub fn intrinsic(name: &str) -> Self {
        let base = name
            .strip_suffix("f32")
            .or_else(|| name.strip_suffix("f64"))
            .unwrap_or(name);
        match base {
            "sqrt" | "sin" | "cos" | "exp" | "exp2" | "log" | "log2" | "log10" | "floor"
            | "ceil" | "trunc" | "fabs" | "pow" | "powi" | "minimum" | "maximum" | "fma"
            | "fmuladd" | "fadd_fast" | "fadd_algebraic" | "fsub_fast" | "fsub_algebraic"
            | "fmul_fast" | "fmul_algebraic" | "fdiv_fast" | "fdiv_algebraic" | "frem_fast"
            | "frem_algebraic" => Self::EagerOnce,
            _ => Self::Materialize,
        }
    }

    /// Rebuilding a constant per slot is intentional; other aggregate producers run once,
    /// even when the repeat count is zero. Scalar `fill` consumes its argument once.
    pub fn repeat(needs_clone: bool, is_constant: bool) -> Self {
        if needs_clone && !is_constant {
            Self::Materialize
        } else {
            Self::EagerOnce
        }
    }

    /// Three-way comparison tests its operands again on the equality branch.
    pub fn comparison(op: BinaryOp) -> Self {
        match op {
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => Self::EagerOnce,
            _ => Self::Materialize,
        }
    }
}

pub(crate) fn captured(value: &Expr) -> bool {
    matches!(
        value,
        Expr::Ident(_)
            | Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
    )
}

/// The two comparison inputs are evaluated eagerly once. The result values are compiler
/// constants representing the three `Ordering` variants.
pub fn three_way(left: Expr, right: Expr, less: Expr, equal: Expr, greater: Expr) -> Expr {
    let captured_inputs = captured(&left) && captured(&right);
    let (l, r) = if captured_inputs {
        (left.clone(), right.clone())
    } else {
        (jsast::id("$cmpL"), jsast::id("$cmpR"))
    };
    let result = jsast::cond(
        jsast::binary(BinOp::Lt, l.clone(), r.clone()),
        less,
        jsast::cond(jsast::binary(BinOp::StrictEq, l, r), equal, greater),
    );
    if captured_inputs {
        result
    } else {
        jsast::call(
            jsast::arrow(vec!["$cmpL".to_owned(), "$cmpR".to_owned()], result),
            vec![left, right],
        )
    }
}

/// Rust's eager boolean operators, with boolean results and no short-circuited operand.
pub fn eager_boolean(op: BinOp, left: Expr, right: Expr) -> Expr {
    jsast::not(jsast::not(jsast::binary(op, left, right)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_and_unknown_intrinsics_require_captured_operands() {
        for name in [
            "minimum_number_nsz_f64",
            "maximum_number_nsz_f32",
            "copysignf64",
            "roundf32",
            "an_unclassified_intrinsic",
        ] {
            assert_eq!(
                OperandEvaluation::intrinsic(name),
                OperandEvaluation::Materialize
            );
        }
        assert_eq!(
            OperandEvaluation::intrinsic("minimumf64"),
            OperandEvaluation::EagerOnce
        );
        assert_eq!(
            OperandEvaluation::intrinsic("sqrtf32"),
            OperandEvaluation::EagerOnce
        );
    }

    #[test]
    fn comparison_preserves_getter_order_count_and_throw_identity() {
        let expression = three_way(
            jsast::member(jsast::id("inputs"), "left"),
            jsast::member(jsast::id("inputs"), "right"),
            jsast::num(-1),
            jsast::num(0),
            jsast::num(1),
        );
        let source = format!(
            r#"
const assert = require('node:assert/strict');
let trace = [], fail = '', left, right;
const sentinel = {{}};
const inputs = {{
    get left() {{ trace.push('left'); if (fail === 'left') throw sentinel; return left; }},
    get right() {{ trace.push('right'); if (fail === 'right') throw sentinel; return right; }}
}};
const compare = () => {};
for (left of [-1, 0, 1]) for (right of [-1, 0, 1]) {{
    fail = ''; trace = [];
    assert.equal(compare(), Math.sign(left - right));
    assert.deepEqual(trace, ['left', 'right']);
    for (fail of ['left', 'right']) {{
        trace = [];
        assert.throws(compare, error => error === sentinel);
        assert.deepEqual(trace, fail === 'left' ? ['left'] : ['left', 'right']);
    }}
}}
"#,
            jsast::expr_to_string(&expression)
        );
        let output = std::process::Command::new("node")
            .args(["-e", &source])
            .output()
            .expect("Node is required by the compiler semantic tests");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
