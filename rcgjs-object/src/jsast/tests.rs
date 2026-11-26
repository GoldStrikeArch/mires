use std::collections::BTreeMap;
use std::ops::ControlFlow;

use super::*;
use crate::opts::EcmaTarget;

fn p(expr: Expr) -> String {
    expr_to_string(&expr)
}

fn params(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn numbers_use_the_shortest_round_tripping_form() {
    assert_eq!(p(num(0)), "0");
    assert_eq!(p(num(3.0)), "3");
    assert_eq!(p(num(1.5)), "1.5");
    assert_eq!(p(num(-7)), "-7");
    assert_eq!(p(num(0.1 + 0.2)), "0.30000000000000004");
    assert_eq!(p(num(f64::NAN)), "NaN");
    assert_eq!(p(num(f64::INFINITY)), "Infinity");
    assert_eq!(p(num(f64::NEG_INFINITY)), "-Infinity");
    assert_eq!(p(num(-0.0)), "-0");
    assert_eq!(p(raw_num("0x7fffffff")), "0x7fffffff");
}

#[test]
fn float_bits_preserve_non_canonical_nan_payloads_and_signs() {
    // Canonical quiet NaNs stay compact `NaN`.
    assert_eq!(p(f32_from_bits(f32::NAN.to_bits())), "NaN");
    assert_eq!(p(f64_from_bits(f64::NAN.to_bits())), "NaN");

    // Non-NaN edge values stay plain numeric literals.
    assert_eq!(p(f32_from_bits((-0.0f32).to_bits())), "-0");
    assert_eq!(p(f64_from_bits((-0.0f64).to_bits())), "-0");
    assert_eq!(p(f32_from_bits(f32::INFINITY.to_bits())), "Infinity");
    assert_eq!(p(f64_from_bits(f64::NEG_INFINITY.to_bits())), "-Infinity");

    // Non-canonical f32 NaNs (payload or negative sign) bit-cast via Uint32Array.of -> Float32Array.
    assert_eq!(
        p(f32_from_bits(0x7fc0_0001)),
        "new Float32Array(Uint32Array.of(2143289345).buffer)[0]"
    );
    assert_eq!(
        p(f32_from_bits((-f32::NAN).to_bits())),
        "new Float32Array(Uint32Array.of(4290772992).buffer)[0]"
    );

    // Non-canonical f64 NaNs use little-endian [lo, hi] u32 halves (ES2018-safe, no BigInt).
    assert_eq!(
        p(f64_from_bits(0x7ff8_0000_0000_0001)),
        "new Float64Array(Uint32Array.of(1, 2146959360).buffer)[0]"
    );
    assert_eq!(
        p(f64_from_bits((-f64::NAN).to_bits())),
        "new Float64Array(Uint32Array.of(0, 4294443008).buffer)[0]"
    );
    let es2018 = expr_to_string_styled_for_target(
        &f64_from_bits(0x7ff8_1234_5678_9abc),
        Style::Compact,
        EcmaTarget::Es2018,
    );
    assert_eq!(
        es2018,
        "new Float64Array(Uint32Array.of(1450744508,2146964020).buffer)[0]"
    );
    assert!(!es2018.contains("BigInt"));
}

#[test]
fn bigint_literals_keep_their_exact_digits() {
    assert_eq!(p(bigint(0u128)), "0n");
    assert_eq!(p(bigint(u64::MAX)), "18446744073709551615n");
    assert_eq!(p(bigint(i64::MIN)), "-9223372036854775808n");
    assert_eq!(
        p(bigint(u128::MAX)),
        "340282366920938463463374607431768211455n"
    );
    // A negative literal is a unary expression, so it is parenthesized where one is.
    assert_eq!(p(binary(BinOp::Sub, id("a"), bigint(-1i128))), "a - -1n");
    assert_eq!(
        p(method_call(bigint(1u32), "toString", vec![])),
        "(1n).toString()"
    );
}

#[test]
fn es2018_spells_bigints_as_runtime_calls() {
    let render =
        |expr: Expr| expr_to_string_styled_for_target(&expr, Style::Readable, EcmaTarget::Es2018);
    assert_eq!(render(bigint(0u32)), "BigInt(\"0\")");
    assert_eq!(render(bigint(i64::MIN)), "BigInt(\"-9223372036854775808\")");
    assert_eq!(
        render(method_call(bigint(1u32), "toString", vec![])),
        "(BigInt(\"1\")).toString()"
    );
}

#[test]
fn bigint_conversions_and_masks() {
    assert_eq!(p(bigint_mask(true, 64, id("a"))), "BigInt.asIntN(64, a)");
    assert_eq!(
        p(bigint_mask(false, 128, to_bigint(id("a")))),
        "BigInt.asUintN(128, BigInt(a))"
    );
    assert_eq!(
        p(to_number(bigint_mask(true, 32, id("a")))),
        "Number(BigInt.asIntN(32, a))"
    );
    assert_eq!(
        p(fround(binary(BinOp::Add, id("a"), id("b")))),
        "Math.fround(a + b)"
    );
}

#[test]
fn strings_escape_quotes_backslashes_and_control_characters() {
    assert_eq!(p(string("hi")), "\"hi\"");
    assert_eq!(p(string("a\"b")), "\"a\\\"b\"");
    assert_eq!(p(string("a\\b")), "\"a\\\\b\"");
    assert_eq!(p(string("a\nb\tc\rd")), "\"a\\nb\\tc\\rd\"");
    assert_eq!(p(string("\u{0}")), "\"\\x00\"");
    assert_eq!(p(string("\u{1b}")), "\"\\x1b\"");
    assert_eq!(p(string("\u{7f}")), "\"\\x7f\"");
    assert_eq!(p(string("\u{8}\u{b}\u{c}")), "\"\\b\\v\\f\"");
    assert_eq!(p(string("a\u{2028}b\u{2029}c")), "\"a\\u2028b\\u2029c\"");
    // Printable non-ASCII stays as is, the output is UTF-8.
    assert_eq!(p(string("caf\u{e9} \u{1f600}")), "\"caf\u{e9} \u{1f600}\"");
}

#[test]
fn literals_and_identifiers() {
    assert_eq!(p(boolean(true)), "true");
    assert_eq!(p(boolean(false)), "false");
    assert_eq!(p(undefined()), "undefined");
    assert_eq!(p(null()), "null");
    assert_eq!(p(id("x")), "x");
}

#[test]
fn member_access_falls_back_to_index_for_awkward_names() {
    assert_eq!(p(member(id("a"), "b")), "a.b");
    assert_eq!(p(member(id("a"), "$t")), "a.$t");
    assert_eq!(p(member(id("a"), "_0")), "a._0");
    assert_eq!(p(member(id("a"), "my field")), "a[\"my field\"]");
    assert_eq!(p(member(id("a"), "0")), "a[\"0\"]");
    assert_eq!(p(member(id("a"), "")), "a[\"\"]");
    // Reserved words are quoted so the output parses everywhere.
    assert_eq!(p(member(id("a"), "default")), "a[\"default\"]");
    assert_eq!(p(member(id("a"), "new")), "a[\"new\"]");
}

#[test]
fn member_of_a_number_literal_is_parenthesized() {
    assert_eq!(p(method_call(num(1), "toString", vec![])), "(1).toString()");
    assert_eq!(p(member(num(1.5), "x")), "(1.5).x");
    assert_eq!(p(member(raw_num("1e3"), "x")), "(1e3).x");
}

#[test]
fn member_of_call_chains_stay_flat() {
    let expr = method_call(method_call(id("a"), "b", vec![num(1)]), "c", vec![id("d")]);
    assert_eq!(p(expr), "a.b(1).c(d)");
    assert_eq!(p(index(call(id("f"), vec![]), num(0))), "f()[0]");
    // A call on an arrow needs the parentheses back.
    assert_eq!(p(call(arrow(vec![], num(1)), vec![])), "(() => 1)()");
}

#[test]
fn arithmetic_precedence_stays_clean() {
    let a = || id("a");
    let b = || id("b");
    let c = || id("c");
    assert_eq!(
        p(binary(BinOp::Add, a(), binary(BinOp::Mul, b(), c()))),
        "a + b * c"
    );
    assert_eq!(
        p(binary(BinOp::Mul, binary(BinOp::Add, a(), b()), c())),
        "(a + b) * c"
    );
    // Left associative, so the left child never needs parentheses.
    assert_eq!(
        p(binary(BinOp::Sub, binary(BinOp::Sub, a(), b()), c())),
        "a - b - c"
    );
    // The right child of the same precedence does.
    assert_eq!(
        p(binary(BinOp::Sub, a(), binary(BinOp::Sub, b(), c()))),
        "a - (b - c)"
    );
    assert_eq!(
        p(binary(BinOp::Div, a(), binary(BinOp::Mul, b(), c()))),
        "a / (b * c)"
    );
}

#[test]
fn mixed_logical_and_bitwise_precedence() {
    let a = || id("a");
    let b = || id("b");
    let c = || id("c");
    // `&&` binds tighter than `||`.
    assert_eq!(
        p(binary(BinOp::Or, binary(BinOp::And, a(), b()), c())),
        "a && b || c"
    );
    assert_eq!(
        p(binary(BinOp::And, binary(BinOp::Or, a(), b()), c())),
        "(a || b) && c"
    );
    // Bitwise binds tighter than logical.
    assert_eq!(
        p(binary(BinOp::BitOr, a(), binary(BinOp::And, b(), c()))),
        "a | (b && c)"
    );
    assert_eq!(
        p(binary(BinOp::And, a(), binary(BinOp::BitOr, b(), c()))),
        "a && b | c"
    );
    // Equality binds tighter than bitwise and.
    assert_eq!(
        p(binary(BinOp::BitAnd, binary(BinOp::Eq, a(), b()), c())),
        "a == b & c"
    );
    assert_eq!(
        p(binary(BinOp::Eq, binary(BinOp::BitAnd, a(), b()), c())),
        "(a & b) == c"
    );
    // Shifts bind looser than addition, which is the classic trap.
    assert_eq!(
        p(binary(BinOp::Shl, binary(BinOp::Add, a(), b()), c())),
        "a + b << c"
    );
    assert_eq!(
        p(binary(BinOp::Add, binary(BinOp::Shl, a(), b()), c())),
        "(a << b) + c"
    );
}

#[test]
fn wrapping_helpers_parenthesize_their_operand() {
    assert_eq!(
        p(i32_wrap(binary(BinOp::Add, id("a"), id("b")))),
        "a + b | 0"
    );
    assert_eq!(
        p(binary(BinOp::Add, i32_wrap(id("a")), id("b"))),
        "(a | 0) + b"
    );
    assert_eq!(p(u32_wrap(id("a"))), "a >>> 0");
    assert_eq!(p(sign_extend(8, id("a"))), "a << 24 >> 24");
    assert_eq!(p(sign_extend(16, id("a"))), "a << 16 >> 16");
    assert_eq!(p(zero_extend(8, id("a"))), "a & 255");
    assert_eq!(p(imul(id("a"), id("b"))), "Math.imul(a, b)");
    assert_eq!(p(i32_wrap(imul(id("a"), id("b")))), "Math.imul(a, b) | 0");
}

#[test]
fn unary_minus_never_forms_a_decrement() {
    assert_eq!(p(neg(id("a"))), "-a");
    assert_eq!(p(neg(neg(id("a")))), "- -a");
    assert_eq!(p(neg(num(-1))), "- -1");
    assert_eq!(p(unary(UnOp::Pos, unary(UnOp::Pos, id("a")))), "+ +a");
    assert_eq!(p(binary(BinOp::Sub, id("a"), neg(id("b")))), "a - -b");
    assert_eq!(p(binary(BinOp::Sub, id("a"), num(-1))), "a - -1");
    assert_eq!(
        p(binary(BinOp::Add, id("a"), unary(UnOp::Pos, id("b")))),
        "a + +b"
    );
    // A negative literal is a unary expression, so it is parenthesized where one is.
    assert_eq!(p(member(id("a"), "b")), "a.b");
    assert_eq!(p(binary(BinOp::Mul, num(-1), id("b"))), "-1 * b");
    assert_eq!(p(neg(binary(BinOp::Add, id("a"), id("b")))), "-(a + b)");
    assert_eq!(p(not(binary(BinOp::Eq, id("a"), id("b")))), "!(a == b)");
    assert_eq!(p(bit_not(id("a"))), "~a");
    assert_eq!(p(type_of(id("a"))), "typeof a");
    assert_eq!(
        p(binary(BinOp::StrictEq, type_of(id("a")), string("number"))),
        "typeof a === \"number\""
    );
}

#[test]
fn nested_ternaries() {
    let a = || id("a");
    let b = || id("b");
    let c = || id("c");
    let d = || id("d");
    let e = || id("e");
    // Right associative: a nested alternate needs no parentheses.
    assert_eq!(p(cond(a(), b(), cond(c(), d(), e()))), "a ? b : c ? d : e");
    // A nested consequent is unambiguous too.
    assert_eq!(p(cond(a(), cond(b(), c(), d()), e())), "a ? b ? c : d : e");
    // A conditional test must be parenthesized.
    assert_eq!(
        p(cond(cond(a(), b(), c()), d(), e())),
        "(a ? b : c) ? d : e"
    );
    // A conditional inside anything tighter gets parentheses.
    assert_eq!(
        p(binary(BinOp::Add, cond(a(), b(), c()), d())),
        "(a ? b : c) + d"
    );
    assert_eq!(
        p(cond(binary(BinOp::Lt, a(), b()), num(1), num(2))),
        "a < b ? 1 : 2"
    );
}

#[test]
fn assignments_are_right_associative_and_nest_under_conditionals() {
    assert_eq!(p(assign(id("a"), id("b"))), "a = b");
    assert_eq!(p(assign(id("a"), assign(id("b"), num(1)))), "a = b = 1");
    assert_eq!(p(assign(member(id("a"), "b"), num(1))), "a.b = 1");
    assert_eq!(p(assign(index(id("a"), num(0)), num(1))), "a[0] = 1");
    assert_eq!(
        p(binary(BinOp::Add, assign(id("a"), num(1)), num(2))),
        "(a = 1) + 2"
    );
    assert_eq!(
        p(cond(id("c"), assign(id("a"), num(1)), num(2))),
        "c ? a = 1 : 2"
    );
    assert_eq!(
        p(compound_assign(CompoundOp::Add, id("a"), num(1))),
        "a += 1"
    );
    assert_eq!(
        p(compound_assign(CompoundOp::UShr, id("a"), num(0))),
        "a >>>= 0"
    );
    assert_eq!(
        p(compound_assign(CompoundOp::Sub, id("a"), num(-1))),
        "a -= -1"
    );
}

#[test]
fn es2018_logical_assignment_snapshots_member_references() {
    let render =
        |expr: Expr| expr_to_string_styled_for_target(&expr, Style::Readable, EcmaTarget::Es2018);
    assert_eq!(
        render(compound_assign(
            CompoundOp::Or,
            id("value"),
            call(id("make"), vec![]),
        )),
        "value || (value = make())"
    );
    assert_eq!(
        render(compound_assign(
            CompoundOp::Or,
            member(call(id("object"), vec![]), "value"),
            call(id("make"), vec![]),
        )),
        "((__rcgjs_es2018_object_0) => __rcgjs_es2018_object_0.value || \
         (__rcgjs_es2018_object_0.value = make()))(object())"
    );
    assert_eq!(
        render(compound_assign(
            CompoundOp::And,
            index(call(id("object"), vec![]), call(id("key"), vec![])),
            call(id("make"), vec![]),
        )),
        "((__rcgjs_es2018_object_0, __rcgjs_es2018_key_0) => \
         __rcgjs_es2018_object_0[__rcgjs_es2018_key_0] && \
         (__rcgjs_es2018_object_0[__rcgjs_es2018_key_0] = make()))(object(), key())"
    );
}

#[test]
fn nullish_assignment_is_es2018_and_not_falsy_assignment() {
    let rendered = p(nullish_assign(
        member(call(id("object"), vec![]), "value"),
        call(id("make"), vec![]),
    ));
    assert!(!rendered.contains("??="), "{rendered}");
    assert!(rendered.contains("=== null"), "{rendered}");
    assert!(rendered.contains("=== void 0"), "{rendered}");
    assert!(!rendered.contains("undefined"), "{rendered}");
    assert_eq!(rendered.matches("object()").count(), 1, "{rendered}");
    assert_eq!(rendered.matches(".value").count(), 2, "{rendered}");
}

#[test]
fn object_keys_are_quoted_only_when_needed() {
    assert_eq!(p(object_of(vec![])), "{}");
    assert_eq!(
        p(object_of(vec![("x", num(1)), ("y", num(2))])),
        "{ x: 1, y: 2 }"
    );
    assert_eq!(p(object_of(vec![("$t", num(0))])), "{ $t: 0 }");
    assert_eq!(
        p(object_of(vec![("$t", num(1)), ("_0", id("a"))])),
        "{ $t: 1, _0: a }"
    );
    assert_eq!(p(object_of(vec![("a b", num(1))])), "{ \"a b\": 1 }");
    assert_eq!(p(object_of(vec![("0", num(1))])), "{ \"0\": 1 }");
    assert_eq!(p(object_of(vec![("class", num(1))])), "{ \"class\": 1 }");
    assert_eq!(
        p(object_of(vec![("a", cond(id("c"), num(1), num(2)))])),
        "{ a: c ? 1 : 2 }"
    );
    // A comma expression in a value position needs parentheses.
    assert_eq!(
        p(object_of(vec![("a", seq(vec![num(1), num(2)]))])),
        "{ a: (1, 2) }"
    );
}

#[test]
fn arrays_and_cells() {
    assert_eq!(p(array(vec![])), "[]");
    assert_eq!(
        p(array(vec![num(1), string("a"), undefined()])),
        "[1, \"a\", undefined]"
    );
    assert_eq!(p(cell(num(0))), "{ v: 0 }");
    assert_eq!(p(cell_get(id("c"))), "c.v");
    assert_eq!(p(assign(cell_get(id("c")), num(1))), "c.v = 1");
}

#[test]
fn arrows_and_accessors() {
    assert_eq!(p(arrow(vec![], num(1))), "() => 1");
    assert_eq!(p(arrow(params(&["x"]), id("x"))), "(x) => x");
    assert_eq!(
        p(arrow(
            params(&["x", "y"]),
            binary(BinOp::Add, id("x"), id("y"))
        )),
        "(x, y) => x + y"
    );
    // An object body has to be parenthesized.
    assert_eq!(
        p(arrow(vec![], object_of(vec![("a", num(1))]))),
        "() => ({ a: 1 })"
    );
    assert_eq!(
        p(accessor(member(id("s"), "field"), "v")),
        "{ get: () => s.field, set: (v) => s.field = v }"
    );
    assert_eq!(
        p(accessor(index(id("t"), num(0)), "v")),
        "{ get: () => t[0], set: (v) => t[0] = v }"
    );
    assert_eq!(p(method_call(id("r"), "get", vec![])), "r.get()");
}

#[test]
fn new_expressions() {
    assert_eq!(
        p(new(id("Error"), vec![string("boom")])),
        "new Error(\"boom\")"
    );
    assert_eq!(p(new(member(id("a"), "B"), vec![])), "new a.B()");
    // A call in the callee would swallow the argument list.
    assert_eq!(p(new(call(id("f"), vec![]), vec![])), "new (f())()");
    assert_eq!(p(member(new(id("A"), vec![]), "x")), "new A().x");
}

#[test]
fn sequences_and_raw() {
    assert_eq!(p(seq(vec![num(1), num(2), num(3)])), "1, 2, 3");
    assert_eq!(
        p(binary(BinOp::Add, seq(vec![num(1), num(2)]), num(3))),
        "(1, 2) + 3"
    );
    assert_eq!(p(raw_expr("a ?? b")), "a ?? b");
    // Raw is always parenthesized when nested, since its precedence is unknown.
    assert_eq!(
        p(binary(BinOp::Add, raw_expr("a ?? b"), num(1))),
        "(a ?? b) + 1"
    );
}

#[test]
fn rt_calls_use_the_shim_namespace() {
    assert_eq!(
        p(rt_call("js_log_i32", vec![id("x")])),
        "__rt.js_log_i32(x)"
    );
    assert_eq!(
        p(rt_call("js_abort", vec![string("nope")])),
        "__rt.js_abort(\"nope\")"
    );
}

#[test]
fn statement_position_hazards_get_parentheses() {
    // An expression statement that starts with a brace reads as a block.
    assert_eq!(
        stmt_to_string(&expr_stmt(object_of(vec![("a", num(1))]))),
        "({ a: 1 });\n"
    );
    assert_eq!(
        stmt_to_string(&expr_stmt(raw_expr("function () {}"))),
        "(function () {});\n"
    );
    // An arrow starts with a paren, so it is already fine.
    assert_eq!(
        stmt_to_string(&expr_stmt(call(arrow(vec![], num(1)), vec![]))),
        "(() => 1)();\n"
    );
    // A name that merely starts with `function` is not a hazard.
    assert_eq!(
        stmt_to_string(&expr_stmt(id("functional"))),
        "functional;\n"
    );
}

#[test]
fn declarations_and_simple_statements() {
    assert_eq!(stmt_to_string(&let_("x", num(1))), "let x = 1;\n");
    assert_eq!(stmt_to_string(&let_uninit("x")), "let x;\n");
    assert_eq!(
        stmt_to_string(&lets(vec![
            ("a".into(), Some(num(1))),
            ("b".into(), None),
            ("c".into(), Some(id("d"))),
        ])),
        "let a = 1, b, c = d;\n"
    );
    assert_eq!(stmt_to_string(&const_("k", num(2))), "const k = 2;\n");
    assert_eq!(stmt_to_string(&ret(id("x"))), "return x;\n");
    assert_eq!(stmt_to_string(&ret_void()), "return;\n");
    assert_eq!(
        stmt_to_string(&Stmt::Return(Some(undefined()))),
        "return undefined;\n"
    );
    assert_eq!(stmt_to_string(&continue_("L")), "continue L;\n");
    assert_eq!(stmt_to_string(&Stmt::Continue(None)), "continue;\n");
    assert_eq!(stmt_to_string(&break_("L")), "break L;\n");
    assert_eq!(stmt_to_string(&Stmt::Break(None)), "break;\n");
    assert_eq!(
        stmt_to_string(&throw_error("unreachable")),
        "throw new Error(\"unreachable\");\n"
    );
    assert_eq!(
        stmt_to_string(&assign_stmt(member(id("a"), "b"), num(1))),
        "a.b = 1;\n"
    );
}

#[test]
fn comments_are_single_line() {
    assert_eq!(stmt_to_string(&comment("bb0")), "// bb0\n");
    assert_eq!(
        stmt_to_string(&comment("first\nsecond\r\nthird")),
        "// first second  third\n"
    );
    assert_eq!(stmt_to_string(&comment("a\u{2028}b")), "// a b\n");
}

#[test]
fn if_else_chains_are_readable() {
    let stmt = if_else(
        binary(BinOp::Lt, id("x"), num(0)),
        vec![ret(num(-1))],
        vec![if_else(
            binary(BinOp::Gt, id("x"), num(0)),
            vec![ret(num(1))],
            vec![ret(num(0))],
        )],
    );
    assert_eq!(
        stmt_to_string(&stmt),
        "\
if (x < 0) {
  return -1;
} else if (x > 0) {
  return 1;
} else {
  return 0;
}
"
    );
}

#[test]
fn empty_bodies_collapse() {
    assert_eq!(stmt_to_string(&if_(id("c"), vec![])), "if (c) {}\n");
    assert_eq!(
        stmt_to_string(&function("f", vec![], vec![])),
        "function f() {}\n"
    );
    assert_eq!(stmt_to_string(&Stmt::Block(vec![])), "{}\n");
}

#[test]
fn while_loops_nest_and_indent() {
    let stmt = while_(
        binary(BinOp::Lt, id("i"), num(10)),
        vec![
            expr_stmt(compound_assign(CompoundOp::Add, id("i"), num(1))),
            if_(binary(BinOp::Eq, id("i"), num(5)), vec![Stmt::Break(None)]),
        ],
    );
    assert_eq!(
        stmt_to_string(&stmt),
        "\
while (i < 10) {
  i += 1;
  if (i == 5) {
    break;
  }
}
"
    );
}

#[test]
fn switch_cases_are_flat_and_support_fallthrough_labels() {
    let stmt = switch(
        id("bb"),
        vec![
            case(vec![num(0)], vec![assign_stmt(id("x"), num(1))]),
            case(
                vec![num(1), num(2)],
                vec![assign_stmt(id("x"), num(2)), continue_("L")],
            ),
            default_case(vec![throw_error("unreachable")]),
        ],
    );
    assert_eq!(
        stmt_to_string(&stmt),
        "\
switch (bb) {
  case 0:
    x = 1;
  case 1:
  case 2:
    x = 2;
    continue L;
  default:
    throw new Error(\"unreachable\");
}
"
    );
}

#[test]
fn labeled_statements_share_the_line() {
    assert_eq!(
        stmt_to_string(&labeled(
            "L",
            while_(boolean(true), vec![Stmt::Break(None)])
        )),
        "L: while (true) {\n  break;\n}\n"
    );
    assert_eq!(
        stmt_to_string(&loop_forever("L", vec![Stmt::Break(None)])),
        "L: for (;;) {\n  break;\n}\n"
    );
    assert_eq!(
        stmt_to_string(&Stmt::LoopForever(None, vec![Stmt::Break(None)])),
        "for (;;) {\n  break;\n}\n"
    );
}

#[test]
fn raw_statements_are_reindented() {
    let stmt = function("f", vec![], vec![raw_stmt("a();\nb();")]);
    assert_eq!(stmt_to_string(&stmt), "function f() {\n  a();\n  b();\n}\n");
}

#[test]
fn identifier_checks_and_sanitizing() {
    assert!(is_plain_ident("a"));
    assert!(is_plain_ident("_0"));
    assert!(is_plain_ident("$t"));
    assert!(is_plain_ident("a$b_1"));
    assert!(!is_plain_ident(""));
    assert!(!is_plain_ident("0a"));
    assert!(!is_plain_ident("a-b"));
    assert!(!is_plain_ident("a b"));
    assert!(!is_plain_ident("class"));

    assert_eq!(sanitize_ident("foo"), "foo");
    assert_eq!(sanitize_ident("my_crate::foo::bar"), "my_crate__foo__bar");
    assert_eq!(sanitize_ident("Foo<i32>::bar"), "Foo_i32___bar");
    assert_eq!(sanitize_ident("0start"), "_0start");
    assert_eq!(sanitize_ident(""), "_");
    assert_eq!(sanitize_ident("class"), "_class");
    assert_eq!(sanitize_ident("foo$1a2b"), "foo$1a2b");
}

#[test]
fn programs_space_out_function_declarations() {
    let program = vec![
        comment("generated"),
        function("a", vec![], vec![ret(num(1))]),
        function("b", vec![], vec![ret(num(2))]),
    ];
    assert_eq!(
        program_to_string(&program),
        "\
// generated
function a() {
  return 1;
}

function b() {
  return 2;
}
"
    );
}

/// A comment introducing a declaration belongs to it: the blank line goes above the comment,
/// never between the comment and the code it describes. This is how every item is printed —
/// `item.rs` renders a `// <def path>` header and the declaration as a two statement program —
/// so getting it wrong labels every emitted function with the name of the one before it.
#[test]
fn a_comment_travels_with_the_declaration_it_introduces() {
    let program = vec![
        function("a", vec![], vec![]),
        comment("about b"),
        comment("still about b"),
        function("b", vec![], vec![]),
        comment("about nothing in particular"),
        let_("c", num(1)),
    ];
    assert_eq!(
        program_to_string(&program),
        "\
function a() {}

// about b
// still about b
function b() {}

// about nothing in particular
let c = 1;
"
    );
}

/// The shape the MIR walk is expected to emit: locals up front, then a labeled forever
/// loop around a switch on the block index.
fn sample_program() -> Vec<Stmt> {
    let bb = || id("bb");
    let sum = || id("sum");
    let i = || id("i");

    let body = vec![
        comment("locals"),
        let_("bb", num(0)),
        let_uninit("sum"),
        let_uninit("i"),
        let_("cell", cell(num(0))),
        let_("acc", accessor(member(id("self"), "count"), "v")),
        loop_forever(
            "L",
            vec![switch(
                bb(),
                vec![
                    case(
                        vec![num(0)],
                        vec![
                            comment("bb0: entry"),
                            assign_stmt(sum(), num(0)),
                            assign_stmt(i(), num(0)),
                            assign_stmt(bb(), num(1)),
                            continue_("L"),
                        ],
                    ),
                    case(
                        vec![num(1)],
                        vec![
                            assign_stmt(
                                bb(),
                                cond(binary(BinOp::Lt, i(), id("n")), num(2), num(3)),
                            ),
                            continue_("L"),
                        ],
                    ),
                    case(
                        vec![num(2)],
                        vec![
                            assign_stmt(
                                sum(),
                                i32_wrap(binary(BinOp::Add, sum(), imul(i(), num(2)))),
                            ),
                            assign_stmt(i(), i32_wrap(binary(BinOp::Add, i(), num(1)))),
                            assign_stmt(bb(), num(1)),
                            continue_("L"),
                        ],
                    ),
                    case(
                        vec![num(3)],
                        vec![
                            expr_stmt(rt_call("js_log_i32", vec![sum()])),
                            expr_stmt(rt_call("js_log_str", vec![string("done \"ok\"\n")])),
                            expr_stmt(method_call(id("acc"), "set", vec![sum()])),
                            assign_stmt(cell_get(id("cell")), neg(num(-1))),
                            ret(sum()),
                        ],
                    ),
                    default_case(vec![throw_error("unreachable basic block")]),
                ],
            )],
        ),
    ];

    vec![
        comment("generated by rustc_codegen_js"),
        function("rust_entry", params(&["n", "self"]), body),
    ]
}

#[test]
fn trampoline_renders_the_expected_shape() {
    let text = program_to_string(&sample_program());
    assert!(text.contains("L: for (;;) {"), "{text}");
    assert!(text.contains("    case 0:\n"), "{text}");
    assert!(text.contains("      bb = 1;\n"), "{text}");
    assert!(text.contains("      continue L;\n"), "{text}");
    assert!(text.contains("bb = i < n ? 2 : 3;"), "{text}");
    assert!(text.contains("sum = sum + Math.imul(i, 2) | 0;"), "{text}");
    assert!(
        text.contains("__rt.js_log_str(\"done \\\"ok\\\"\\n\");"),
        "{text}"
    );
    assert!(text.contains("cell.v = - -1;"), "{text}");
    assert!(
        text.contains("let acc = { get: () => self.count, set: (v) => self.count = v };"),
        "{text}"
    );
}

/// Proves the printer emits syntactically valid JavaScript. Skipped when node is absent.
#[test]
fn sample_program_passes_node_check() {
    let text = program_to_string(&sample_program());
    let path = std::env::temp_dir().join("jsast_sample_program.js");
    if std::fs::write(&path, &text).is_err() {
        return;
    }
    let output = match std::process::Command::new("node")
        .arg("--check")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        // No node on this machine, nothing to prove.
        Err(_) => return,
    };
    assert!(
        output.status.success(),
        "node --check failed:\n{}\n--- source ---\n{text}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A grab bag of the printer's corners, checked by node rather than by eye.
#[test]
fn tricky_expressions_pass_node_check() {
    let exprs = vec![
        neg(neg(id("a"))),
        binary(BinOp::Sub, id("a"), num(-1)),
        method_call(num(1), "toString", vec![]),
        cond(cond(id("a"), id("b"), id("c")), id("d"), id("e")),
        binary(BinOp::BitOr, id("a"), binary(BinOp::And, id("b"), id("c"))),
        object_of(vec![("class", num(1)), ("a b", num(2)), ("$t", num(3))]),
        member(id("a"), "default"),
        arrow(vec![], object_of(vec![("a", num(1))])),
        accessor(member(id("s"), "field"), "v"),
        new(call(id("f"), vec![]), vec![]),
        seq(vec![num(1), num(2)]),
        string("quotes \" backslash \\ newline \n tab \t nul \u{0} unicode \u{2028}"),
        call(arrow(vec![], num(1)), vec![]),
        binary(BinOp::Add, assign(id("a"), num(1)), num(2)),
        i32_wrap(binary(BinOp::Add, id("a"), id("b"))),
        sign_extend(8, binary(BinOp::Add, id("a"), id("b"))),
    ];
    let mut program = vec![let_uninit("a"), let_uninit("b"), let_uninit("c")];
    program.push(let_uninit("d"));
    program.push(let_uninit("e"));
    program.push(let_uninit("s"));
    program.push(let_uninit("f"));
    for expr in exprs {
        program.push(expr_stmt(expr));
    }
    let text = program_to_string(&program);
    let path = std::env::temp_dir().join("jsast_tricky_exprs.js");
    if std::fs::write(&path, &text).is_err() {
        return;
    }
    let output = match std::process::Command::new("node")
        .arg("--check")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(_) => return,
    };
    assert!(
        output.status.success(),
        "node --check failed:\n{}\n--- source ---\n{text}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs a program through node and compares stdout, so the precedence table is checked
/// against real evaluation and not just against expected text.
#[test]
fn precedence_matches_node_evaluation() {
    let cases: Vec<(Expr, &str)> = vec![
        (
            binary(BinOp::Add, num(1), binary(BinOp::Mul, num(2), num(3))),
            "7",
        ),
        (
            binary(BinOp::Mul, binary(BinOp::Add, num(1), num(2)), num(3)),
            "9",
        ),
        (
            binary(BinOp::Sub, num(1), binary(BinOp::Sub, num(2), num(3))),
            "2",
        ),
        (binary(BinOp::Sub, num(1), num(-2)), "3"),
        (neg(neg(num(3))), "3"),
        (
            binary(BinOp::Add, binary(BinOp::Shl, num(1), num(2)), num(1)),
            "5",
        ),
        (
            binary(BinOp::Shl, binary(BinOp::Add, num(1), num(2)), num(1)),
            "6",
        ),
        (
            binary(BinOp::BitOr, num(1), binary(BinOp::And, num(0), num(2))),
            "1",
        ),
        (
            cond(boolean(false), num(1), cond(boolean(true), num(2), num(3))),
            "2",
        ),
        (
            i32_wrap(binary(BinOp::Add, raw_num("2147483647"), num(1))),
            "-2147483648",
        ),
        (u32_wrap(neg(num(1))), "4294967295"),
        (sign_extend(8, num(255)), "-1"),
        (imul(num(65536), num(65536)), "0"),
        (member(object_of(vec![("class", num(7))]), "class"), "7"),
        (method_call(num(255), "toString", vec![num(16)]), "ff"),
    ];

    let mut program = Vec::new();
    for (expr, _) in &cases {
        program.push(expr_stmt(method_call(
            id("console"),
            "log",
            vec![expr.clone()],
        )));
    }
    let text = program_to_string(&program);
    let path = std::env::temp_dir().join("jsast_precedence_eval.js");
    if std::fs::write(&path, &text).is_err() {
        return;
    }
    let output = match std::process::Command::new("node").arg(&path).output() {
        Ok(output) => output,
        Err(_) => return,
    };
    assert!(
        output.status.success(),
        "node failed:\n{}\n--- source ---\n{text}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), cases.len(), "{stdout}");
    for (line, (expr, expected)) in lines.iter().zip(&cases) {
        assert_eq!(line, expected, "for `{}`", expr_to_string(expr));
    }
}

/// The marker is invisible in the text, which is what lets `js-source-map=on` describe the
/// output without changing it. Everything below prints exactly as it does without the markers.
#[test]
fn location_markers_print_nothing() {
    let program = vec![function(
        "f",
        vec![],
        vec![
            loc("a.rs", 1, 0),
            let_("x", num(1)),
            loc("a.rs", 2, 4),
            loc("a.rs", 2, 4),
            ret(id("x")),
        ],
    )];
    assert_eq!(
        program_to_string(&program),
        "function f() {\n  let x = 1;\n  return x;\n}\n"
    );
    // Even an `else if` chain, which is a pattern match over the branch it prints.
    let chain = if_else(
        id("a"),
        vec![ret(num(1))],
        vec![loc("a.rs", 9, 2), if_(id("b"), vec![ret(num(2))])],
    );
    assert_eq!(
        stmt_to_string(&chain),
        "if (a) {\n  return 1;\n} else if (b) {\n  return 2;\n}\n"
    );
}

#[test]
fn mappings_point_at_the_statements_the_markers_introduced() {
    let program = vec![function(
        "f$h00",
        vec![],
        vec![
            loc("a.rs", 7, 4),
            let_("acc$1", num(1)),
            loc("a.rs", 8, 4),
            ret(id("acc$1")),
        ],
    )];
    let names = BTreeMap::from([
        ("f$h00".to_string(), "the_fn".to_string()),
        ("acc$1".to_string(), "acc".to_string()),
    ]);
    let (text, mappings) = program_to_string_mapped(
        &program,
        names,
        Some(Loc {
            file: "a.rs".into(),
            line: 6,
            col: 0,
        }),
    );
    assert_eq!(
        text,
        "function f$h00() {\n  let acc$1 = 1;\n  return acc$1;\n}\n"
    );

    let at = |mapping: &Mapping| {
        (
            mapping.gen_line,
            mapping.gen_col,
            mapping.loc.line,
            mapping.loc.col,
            mapping.name.clone(),
        )
    };
    assert_eq!(
        mappings.iter().map(at).collect::<Vec<_>>(),
        vec![
            // The declaration itself, from the location the item carried in.
            (0, 0, 6, 0, None),
            // `f$h00` at column 9, named through the opening parenthesis at column 14, and a
            // fresh mapping right after it so that the name does not bleed onto the
            // parameter list.
            (0, 9, 6, 0, Some("the_fn".to_string())),
            (0, 15, 6, 0, None),
            // The `let`, then its declared name.
            (1, 2, 7, 4, None),
            (1, 6, 7, 4, Some("acc".to_string())),
            (1, 11, 7, 4, None),
            (2, 2, 8, 4, None),
        ]
    );
}

/// The column a mapping carries counts UTF-16 code units, not bytes: that is the unit the
/// format is defined in and the one a browser measures the line with.
#[test]
fn generated_columns_are_counted_in_utf16_units() {
    let program = vec![
        Stmt::Loc(Loc {
            file: "a.rs".into(),
            line: 3,
            col: 1,
        }),
        lets(vec![
            ("a".to_string(), Some(string("caf\u{e9}\u{1f600}"))),
            ("b".to_string(), None),
        ]),
    ];
    let names = BTreeMap::from([("b".to_string(), "beta".to_string())]);
    let (text, mappings) = program_to_string_mapped(&program, names, None);
    assert_eq!(text, "let a = \"caf\u{e9}\u{1f600}\", b;\n");

    // `b` is the 19th UTF-16 unit of the line: the astral emoji before it counts as two, and
    // the accented letter as one, where the byte offset would say 23.
    let named = mappings
        .iter()
        .find(|m| m.name.is_some())
        .expect("a named mapping");
    assert_eq!((named.gen_line, named.gen_col), (0, 18));
}

#[test]
fn visit_idents_sees_references_but_not_names() {
    // A function that calls another item, reads a property of the same name as a third item,
    // and builds an object whose key looks like an item too.
    let decl = function(
        "caller",
        vec!["_1".to_string()],
        vec![
            let_("_0", call(id("callee"), vec![id("_1")])),
            expr_stmt(assign(member(id("_0"), "callee"), num(1))),
            expr_stmt(assign(
                id("_0"),
                object(vec![("callee".to_string(), id("other"))]),
            )),
            ret(id("_0")),
        ],
    );

    let mut seen = Vec::new();
    visit_idents(&decl, &mut |name| seen.push(name.to_owned()));

    // `caller` (its own name), `_0` (a declaration) and the two `callee` *keys* are absent;
    // the two genuine references are present.
    assert_eq!(seen, vec!["callee", "_1", "_0", "_0", "other", "_0"]);
    assert!(!seen.contains(&"caller".to_string()));
}

#[test]
fn free_identifiers_respect_lexical_scopes_and_closure_captures() {
    let decl = function(
        "caller",
        vec!["parameter".into()],
        vec![
            expr_stmt(id("log")),
            Stmt::Block(vec![
                expr_stmt(id("log")),
                let_("log", id("initializer")),
                expr_stmt(arrow(vec![], id("log"))),
            ]),
            Stmt::TryCatch {
                body: vec![expr_stmt(id("caught"))],
                binding: "caught".into(),
                handler: vec![expr_stmt(arrow(vec![], id("caught")))],
            },
            expr_stmt(id("caught")),
            switch(
                id("shared"),
                vec![
                    case(vec![id("shared")], vec![expr_stmt(id("shared"))]),
                    case(vec![num(2)], vec![const_("shared", num(3))]),
                ],
            ),
            expr_stmt(id("shared")),
            while_(
                id("scoped"),
                vec![let_("scoped", num(1)), expr_stmt(id("scoped"))],
            ),
            expr_stmt(id("scoped")),
            expr_stmt(arrow(vec!["shadow".into()], id("shadow"))),
            expr_stmt(id("shadow")),
            function(
                "nested",
                vec![],
                vec![
                    expr_stmt(id("parameter")),
                    expr_stmt(id("nested")),
                    expr_stmt(id("sibling")),
                    expr_stmt(id("arguments")),
                    ret(id("external")),
                ],
            ),
            function("sibling", vec![], vec![ret(id("caller"))]),
            ret(id("nested")),
        ],
    );
    let mut seen = Vec::new();
    visit_free_idents(&decl, &mut |name| seen.push(name.to_owned()));
    assert_eq!(
        seen,
        [
            "log",
            "initializer",
            "caught",
            "caught",
            "shared",
            "shared",
            "scoped",
            "scoped",
            "shadow",
            "external"
        ]
    );
}

#[test]
fn free_identifiers_predeclare_imports_and_block_bindings() {
    let decl = Stmt::Block(vec![
        expr_stmt(id("namespace")),
        expr_stmt(id("local")),
        expr_stmt(id("second")),
        import_namespace("namespace", "./one.js"),
        import_named(vec![("exported".into(), "local".into())], "./two.js"),
        lets(vec![
            ("first".into(), Some(id("second"))),
            ("second".into(), Some(id("input"))),
        ]),
        if_else(
            id("condition"),
            vec![const_("branch", num(1)), ret(id("branch"))],
            vec![ret(id("branch"))],
        ),
        expr_stmt(arrow(vec![], id("arguments"))),
    ]);
    let mut seen = Vec::new();
    visit_free_idents(&decl, &mut |name| seen.push(name.to_owned()));
    assert_eq!(seen, ["input", "condition", "branch", "arguments"]);
}

#[test]
fn visit_bindings_sees_function_local_and_nested_names() {
    let declaration = function(
        "caller",
        vec!["argument".to_owned()],
        vec![
            let_("local", arrow(vec!["nested".to_owned()], id("nested"))),
            const_("constant", id("argument")),
            loop_forever(
                "label",
                vec![lets(vec![
                    ("first".to_owned(), None),
                    ("second".to_owned(), None),
                ])],
            ),
        ],
    );

    let mut seen = Vec::new();
    visit_bindings(&declaration, &mut |name| seen.push(name.to_owned()));

    assert_eq!(
        seen,
        ["caller", "argument", "local", "nested", "constant", "first", "second"]
    );
    assert!(!seen.contains(&"label".to_owned()));
}

#[test]
fn visit_idents_reaches_into_every_nested_position() {
    let decl = function(
        "f",
        vec![],
        vec![
            if_else(
                id("a"),
                vec![switch(
                    id("b"),
                    vec![case(vec![id("c")], vec![expr_stmt(id("d"))])],
                )],
                vec![while_(id("e"), vec![throw(id("g"))])],
            ),
            expr_stmt(arrow_block(vec![], vec![expr_stmt(id("h"))])),
            expr_stmt(arrow(vec![], cond(id("i"), id("j"), id("k")))),
            expr_stmt(array(vec![index(id("l"), id("m"))])),
        ],
    );

    let mut seen = Vec::new();
    visit_idents(&decl, &mut |name| seen.push(name.to_owned()));
    assert_eq!(
        seen,
        ["a", "b", "c", "d", "e", "g", "h", "i", "j", "k", "l", "m"]
    );
}

// ---------------------------------------------------------------------------------------
// Imports (`-Cllvm-args=js-modules=esm`)
// ---------------------------------------------------------------------------------------

#[test]
fn an_import_prints_both_specifier_styles() {
    let namespace = import_namespace("__rt", "./shim.js");
    assert_eq!(
        stmt_to_string(&namespace),
        "import * as __rt from \"./shim.js\";\n"
    );

    let named = import_named(
        vec![
            ("insert".to_string(), "_$insert".to_string()),
            ("template".to_string(), "template".to_string()),
        ],
        "topcoat-dom",
    );
    assert_eq!(
        stmt_to_string(&named),
        "import { insert as _$insert, template } from \"topcoat-dom\";\n"
    );
}

#[test]
fn an_import_elides_the_alias_of_a_binding_that_keeps_its_name() {
    let named = import_named(
        vec![("effect".to_string(), "effect".to_string())],
        "./dom.js",
    );
    assert_eq!(
        stmt_to_string(&named),
        "import { effect } from \"./dom.js\";\n"
    );
}

#[test]
fn a_compact_import_keeps_only_the_separators_it_needs() {
    let namespace = import_namespace("__rt", "./shim.js");
    assert_eq!(compact(&namespace), "import*as __rt from\"./shim.js\";");

    let named = import_named(
        vec![
            ("a".to_string(), "a".to_string()),
            ("b".to_string(), "c".to_string()),
        ],
        "m",
    );
    assert_eq!(compact(&named), "import{a,b as c}from\"m\";");
}

/// A compact program still ends every top level declaration with a newline, an import
/// included: an object file's `//# rcgjs:` footer has to start a line of its own.
#[test]
fn a_compact_program_keeps_a_newline_after_an_import() {
    let program = [
        import_namespace("__rt", "./shim.js"),
        function("f", vec![], vec![ret(num(1))]),
    ];
    assert_eq!(
        program_to_string_styled(&program, Style::Compact),
        "import*as __rt from\"./shim.js\";\nfunction f(){return 1}\n"
    );
}

/// An import is a declaration, so a readable program spaces it out from what follows.
#[test]
fn a_readable_program_spaces_an_import_out_like_any_declaration() {
    let program = [
        import_namespace("__rt", "./shim.js"),
        import_named(vec![("x".to_string(), "x".to_string())], "./x.js"),
        function("f", vec![], vec![]),
    ];
    assert_eq!(
        program_to_string(&program),
        "import * as __rt from \"./shim.js\";\n\nimport { x } from \"./x.js\";\n\n\
         function f() {}\n"
    );
}

/// The specifiers are declarations, not references. An import that reported them would name
/// itself, and `link.rs` would root every import ever emitted instead of dropping the unused
/// ones.
#[test]
fn an_import_mentions_no_identifier() {
    let mut seen: Vec<String> = Vec::new();
    for import in [
        import_namespace("__rt", "./shim.js"),
        import_named(vec![("a".to_string(), "b".to_string())], "m"),
    ] {
        visit_idents(&import, &mut |name| seen.push(name.to_owned()));
    }
    assert!(seen.is_empty(), "an import reported {seen:?}");
}

#[test]
fn an_import_source_is_escaped_like_any_string() {
    let import = import_namespace("m", "./a\"b.js");
    assert_eq!(
        stmt_to_string(&import),
        "import * as m from \"./a\\\"b.js\";\n"
    );
}

// ---------------------------------------------------------------------------------------
// Compact printing (`-Cllvm-args=js-minify`)
// ---------------------------------------------------------------------------------------

fn compact(stmt: &Stmt) -> String {
    stmt_to_string_styled(stmt, Style::Compact)
}

#[test]
fn compact_output_drops_the_whitespace_that_only_separates_tokens() {
    let decl = function(
        "f",
        params(&["a", "b"]),
        vec![
            let_("c", binary(BinOp::Add, id("a"), id("b"))),
            assign_stmt(id("c"), cond(id("a"), num(1), num(2))),
            ret(id("c")),
        ],
    );
    assert_eq!(
        compact(&decl),
        "function f(a,b){let c=a+b;c=a?1:2;return c}"
    );
}

#[test]
fn compact_output_puts_a_space_back_where_tokens_would_fuse() {
    // `a - -1` must not become `a--1`, and `return a` must not become `returna`.
    assert_eq!(
        compact(&expr_stmt(binary(BinOp::Sub, id("a"), num(-1)))),
        "a- -1;"
    );
    assert_eq!(compact(&expr_stmt(neg(neg(id("a"))))), "- -a;");
    assert_eq!(compact(&ret(id("a"))), "return a;");
    // ... but only where they would: a `return` of something that starts with punctuation
    // needs nothing.
    assert_eq!(compact(&ret(num(-1))), "return-1;");
    assert_eq!(
        compact(&throw(new(id("Error"), vec![string("x")]))),
        "throw new Error(\"x\");"
    );
    assert_eq!(compact(&expr_stmt(type_of(id("a")))), "typeof a;");
}

#[test]
fn compact_output_elides_the_semicolon_before_a_brace() {
    let decl = function("f", vec![], vec![assign_stmt(id("a"), num(1))]);
    assert_eq!(compact(&decl), "function f(){a=1}");
}

#[test]
fn compact_output_drops_braces_only_around_a_statement_that_cannot_hold_an_if() {
    assert_eq!(
        compact(&if_(id("a"), vec![assign_stmt(id("b"), num(1))])),
        "if(a)b=1;"
    );
    assert_eq!(
        compact(&if_else(id("a"), vec![ret(num(1))], vec![ret(num(2))])),
        "if(a){return 1}else return 2;"
    );
    // A `let` is not a statement the grammar allows there, and an `if` would catch the `else`.
    assert_eq!(
        compact(&if_(id("a"), vec![let_("b", num(1))])),
        "if(a){let b=1}"
    );
    assert_eq!(
        compact(&if_else(
            id("a"),
            vec![if_(id("b"), vec![assign_stmt(id("c"), num(1))])],
            vec![assign_stmt(id("d"), num(2))],
        )),
        "if(a){if(b)c=1}else d=2;"
    );
    // An `else if` chain stays a chain.
    assert_eq!(
        compact(&if_else(
            id("a"),
            vec![ret(num(1))],
            vec![if_else(id("b"), vec![ret(num(2))], vec![ret(num(3))])],
        )),
        "if(a){return 1}else if(b){return 2}else return 3;"
    );
}

#[test]
fn compact_output_spells_the_short_literals() {
    assert_eq!(compact(&expr_stmt(boolean(true))), "!0;");
    assert_eq!(compact(&expr_stmt(boolean(false))), "!1;");
    assert_eq!(compact(&expr_stmt(undefined())), "void 0;");
    // `!0` binds like a unary expression, so an access over one gains parentheses.
    assert_eq!(compact(&expr_stmt(member(boolean(true), "x"))), "(!0).x;");
    assert_eq!(compact(&expr_stmt(member(undefined(), "x"))), "(void 0).x;");
}

#[test]
fn compact_output_unwraps_a_single_arrow_parameter() {
    assert_eq!(compact(&expr_stmt(arrow(params(&["v"]), id("v")))), "v=>v;");
    assert_eq!(
        compact(&expr_stmt(arrow(params(&["a", "b"]), id("a")))),
        "(a,b)=>a;"
    );
    assert_eq!(compact(&expr_stmt(arrow(vec![], num(1)))), "()=>1;");
    assert_eq!(
        compact(&expr_stmt(accessor(member(id("s"), "f"), "v"))),
        "({get:()=>s.f,set:v=>s.f=v});"
    );
}

#[test]
fn compact_output_keeps_a_newline_per_top_level_item() {
    let program = [
        function("f", vec![], vec![ret(num(1))]),
        let_("g", num(2)),
        function("h", vec![], vec![]),
    ];
    assert_eq!(
        program_to_string_styled(&program, Style::Compact),
        "function f(){return 1}\nlet g=2;\nfunction h(){}\n"
    );
}

#[test]
fn compact_output_omits_comments_entirely() {
    let program = [comment("a header"), function("f", vec![], vec![])];
    assert_eq!(
        program_to_string_styled(&program, Style::Compact),
        "function f(){}\n"
    );
}

/// The real check: the same program, printed both ways, has to *do* the same thing.
///
/// Every rule compact printing adds is a rule about what may be left out, and the only way to
/// be sure none of them changed a meaning is to run both. The cases are chosen for the hazards:
/// dangling `else`, semicolon elision at the end of every kind of block, the `- -` and `+ +`
/// fusions, `!0`/`void 0` in positions where precedence matters, labelled loops.
#[test]
fn compact_and_readable_programs_evaluate_the_same() {
    let log = |expr: Expr| expr_stmt(method_call(id("console"), "log", vec![expr]));
    let program = vec![
        function(
            "f",
            params(&["a", "b"]),
            vec![
                let_("c", binary(BinOp::Sub, id("a"), num(-1))),
                Stmt::If(
                    binary(BinOp::Gt, id("c"), num(0)),
                    vec![Stmt::If(id("b"), vec![assign_stmt(id("c"), num(10))], None)],
                    Some(vec![assign_stmt(id("c"), num(20))]),
                ),
                labeled(
                    "outer",
                    while_(
                        binary(BinOp::Lt, id("c"), num(30)),
                        vec![
                            Stmt::If(
                                binary(BinOp::Eq, id("c"), num(25)),
                                vec![Stmt::Break(Some("outer".to_string()))],
                                None,
                            ),
                            expr_stmt(compound_assign(CompoundOp::Add, id("c"), num(1))),
                        ],
                    ),
                ),
                switch(
                    id("c"),
                    vec![
                        case(vec![num(25)], vec![assign_stmt(id("c"), num(100))]),
                        default_case(vec![assign_stmt(id("c"), num(200))]),
                    ],
                ),
                ret(id("c")),
            ],
        ),
        log(call(id("f"), vec![num(1), boolean(true)])),
        log(call(id("f"), vec![num(1), boolean(false)])),
        log(call(id("f"), vec![num(-5), boolean(true)])),
        log(unary(UnOp::Neg, neg(num(3)))),
        log(binary(BinOp::Add, num(1), unary(UnOp::Pos, num(2)))),
        log(cond(boolean(false), undefined(), boolean(true))),
        log(type_of(undefined())),
        log(object_of(vec![("a", boolean(true)), ("b", undefined())])),
        log(array(vec![boolean(false), num(-1), string("x\"y")])),
        log(call(
            arrow(params(&["v"]), binary(BinOp::Mul, id("v"), num(2))),
            vec![num(21)],
        )),
    ];

    let readable = program_to_string(&program);
    let compact = program_to_string_styled(&program, Style::Compact);
    assert!(
        !compact.contains("\n  "),
        "compact output is indented:\n{compact}"
    );

    let run = |text: &str, name: &str| -> Option<String> {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, text).ok()?;
        let output = std::process::Command::new("node")
            .arg(&path)
            .output()
            .ok()?;
        assert!(
            output.status.success(),
            "node failed:\n{}\n--- source ---\n{text}",
            String::from_utf8_lossy(&output.stderr)
        );
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    };

    let (Some(a), Some(b)) = (
        run(&readable, "jsast_style_readable.js"),
        run(&compact, "jsast_style_compact.js"),
    ) else {
        return;
    };
    assert_eq!(
        a, b,
        "--- readable ---\n{readable}\n--- compact ---\n{compact}"
    );
}

#[test]
fn loop_views_unify_loop_forever_while_and_labeled_loops() {
    let mut forever = loop_forever("L1", vec![Stmt::Break(Some("L1".into()))]);
    let r = forever.as_loop().unwrap();
    assert_eq!(r.label, Some("L1"));
    assert!(r.cond.is_none());
    assert_eq!(r.body.len(), 1);
    forever.as_loop_mut().unwrap().body.push(ret_void());
    assert_eq!(forever.as_loop().unwrap().body.len(), 2);

    let mut wh = while_(id("ok"), vec![ret(num(1))]);
    let r = wh.as_loop().unwrap();
    assert_eq!(r.label, None);
    assert_eq!(r.cond, Some(&id("ok")));
    *wh.as_loop_mut().unwrap().cond.unwrap() = boolean(false);
    assert_eq!(wh.as_loop().unwrap().cond, Some(&boolean(false)));

    let mut labeled_wh = labeled("L2", while_(id("c"), vec![]));
    let r = labeled_wh.as_loop().unwrap();
    assert_eq!(r.label, Some("L2"));
    assert_eq!(r.cond, Some(&id("c")));
    labeled_wh
        .as_loop_mut()
        .unwrap()
        .body
        .push(Stmt::Break(None));
    assert_eq!(labeled_wh.as_loop().unwrap().body.len(), 1);

    let labeled_forever = labeled("L3", Stmt::LoopForever(None, vec![]));
    assert_eq!(labeled_forever.as_loop().unwrap().label, Some("L3"));
    assert!(if_(id("x"), vec![]).as_loop().is_none());
}

#[test]
fn visit_block_mut_and_try_visit_cover_nested_blocks_and_short_circuit() {
    struct DropComments;
    impl VisitMut for DropComments {
        fn visit_block_mut(&mut self, stmts: &mut Vec<Stmt>) {
            self.visit_stmts_mut(stmts);
            stmts.retain(|s| !matches!(s, Stmt::Comment(_)));
        }
    }

    let mut decl = function(
        "f",
        vec![],
        vec![
            comment("top"),
            if_(id("a"), vec![comment("then"), ret(num(1))]),
            expr_stmt(arrow_block(vec![], vec![comment("arrow"), ret(num(2))])),
        ],
    );
    DropComments.visit_stmt_mut(&mut decl);
    let rendered = stmt_to_string(&decl);
    assert!(!rendered.contains("//"), "{rendered}");

    let mut visited_blocks = 0usize;
    walk_stmt_blocks_mut(&mut decl, &mut |_| visited_blocks += 1);
    assert_eq!(visited_blocks, 1);

    let mut count = 0usize;
    let found = try_visit_expr(
        &binary(
            BinOp::Add,
            binary(BinOp::Add, id("first"), id("needle")),
            id("never_visited"),
        ),
        |e| {
            count += 1;
            if matches!(e, Expr::Ident(n) if n == "needle") {
                ControlFlow::Break("hit")
            } else {
                ControlFlow::Continue(())
            }
        },
    );
    assert_eq!(found, ControlFlow::Break("hit"));
    assert_eq!(count, 4);
    assert!(any_expr_in_stmts(
        std::slice::from_ref(&decl),
        |e| matches!(e, Expr::Num(n) if *n == 2.0)
    ));
    assert_eq!(CompoundOp::try_from(BinOp::Add), Ok(CompoundOp::Add));
    assert_eq!(CompoundOp::try_from(BinOp::StrictEq), Err(()));
    assert_eq!(BinOp::from(CompoundOp::UShr), BinOp::UShr);
}
