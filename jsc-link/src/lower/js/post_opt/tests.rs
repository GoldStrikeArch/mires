use std::collections::HashSet;

use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp};

use super::super::i64_pair;
use super::coalesce::{body_calls_self, coalesce_disjoint_locals_with_threshold};
use super::*;

#[test]
fn folds_const_array_index_and_const_switch() {
    let mut decl = Stmt::FunctionDecl {
        name: "f".to_owned(),
        params: vec!["x".to_owned()],
        body: vec![
            jsast::lets(vec![("k".to_owned(), None), ("r".to_owned(), None)]),
            jsast::assign_stmt(jsast::id("k"), Expr::Str("Some".to_owned())),
            Stmt::Switch(
                jsast::id("k"),
                vec![
                    jsast::SwitchCase {
                        tests: vec![Expr::Str("None".to_owned())],
                        body: vec![
                            jsast::assign_stmt(jsast::id("r"), Expr::Num(0.0)),
                            Stmt::Break(None),
                        ],
                    },
                    jsast::SwitchCase {
                        tests: vec![Expr::Str("Some".to_owned())],
                        body: vec![
                            jsast::assign_stmt(
                                jsast::id("r"),
                                Expr::Index(
                                    Box::new(Expr::Array(vec![Expr::Num(1.0)])),
                                    Box::new(Expr::Num(0.0)),
                                ),
                            ),
                            Stmt::Break(None),
                        ],
                    },
                ],
            ),
            Stmt::Return(Some(jsast::id("r"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        !printed.contains("switch"),
        "expected switch folded: {printed}"
    );
    assert!(
        !printed.contains("[1][0]"),
        "expected [1][0] folded: {printed}"
    );
    assert!(
        printed.contains("return 1;"),
        "expected return 1;: {printed}"
    );
}

#[test]
fn optimizes_reflect_get_with_dominated_guard_cse() {
    let mut decl = Stmt::FunctionDecl {
        name: "f".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("k".to_owned(), None),
                ("x".to_owned(), None),
                ("y".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("k"), Expr::Str("children".to_owned())),
            jsast::assign_stmt(
                jsast::id("x"),
                jsast::method_call(
                    jsast::id("Reflect"),
                    "get",
                    vec![jsast::id("a"), jsast::id("k")],
                ),
            ),
            jsast::assign_stmt(
                jsast::id("y"),
                jsast::method_call(
                    jsast::id("Reflect"),
                    "get",
                    vec![jsast::id("a"), Expr::Str("name".to_owned())],
                ),
            ),
            Stmt::Return(Some(jsast::id("y"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        printed.contains("a.children"),
        "expected a.children: {printed}"
    );
    assert!(
        printed.contains("y = a.name;"),
        "expected unguarded dominated a.name: {printed}"
    );
    assert_eq!(
        printed.matches("Reflect.get").count(),
        1,
        "expected only 1 fallback guard: {printed}"
    );
}

#[test]
fn fuses_adjacent_i64_rem_and_div() {
    let mut decl = Stmt::FunctionDecl {
        name: "f".to_owned(),
        params: vec![
            "r".to_owned(),
            "q".to_owned(),
            "e".to_owned(),
            "d".to_owned(),
        ],
        body: vec![
            jsast::lets(vec![("rl".to_owned(), None), ("rh".to_owned(), None)]),
            jsast::assign_stmt(
                jsast::id("rl"),
                jsast::call(
                    jsast::id(i64_pair::HELPER_DIV),
                    vec![
                        jsast::id("r"),
                        jsast::id("q"),
                        jsast::id("e"),
                        jsast::id("d"),
                        Expr::Bool(false),
                        Expr::Bool(true),
                    ],
                ),
            ),
            jsast::assign_stmt(jsast::id("rh"), jsast::id(i64_pair::HELPER_HI)),
            jsast::assign_stmt(
                jsast::id("r"),
                jsast::call(
                    jsast::id(i64_pair::HELPER_DIV),
                    vec![
                        jsast::id("r"),
                        jsast::id("q"),
                        jsast::id("e"),
                        jsast::id("d"),
                        Expr::Bool(false),
                        Expr::Bool(false),
                    ],
                ),
            ),
            jsast::assign_stmt(jsast::id("q"), jsast::id(i64_pair::HELPER_HI)),
            Stmt::Return(Some(jsast::id("rl"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        printed.contains(i64_pair::HELPER_UDIVREM),
        "expected specialized udivrem: {printed}"
    );
    assert!(
        !printed.contains("$__mires_i64_div("),
        "expected both div calls fused: {printed}"
    );
}

#[test]
fn fuses_specialized_i64_srem_sdiv_and_urem_udiv() {
    let mut decl = Stmt::FunctionDecl {
        name: "f".to_owned(),
        params: vec![
            "al".to_owned(),
            "ah".to_owned(),
            "bl".to_owned(),
            "bh".to_owned(),
        ],
        body: vec![
            jsast::lets(vec![
                ("rl".to_owned(), None),
                ("rh".to_owned(), None),
                ("ql".to_owned(), None),
                ("qh".to_owned(), None),
                ("url".to_owned(), None),
                ("urh".to_owned(), None),
                ("uql".to_owned(), None),
                ("uqh".to_owned(), None),
            ]),
            jsast::assign_stmt(
                jsast::id("rl"),
                jsast::call(
                    jsast::id(i64_pair::HELPER_SREM),
                    vec![
                        jsast::id("al"),
                        jsast::id("ah"),
                        jsast::id("bl"),
                        jsast::id("bh"),
                    ],
                ),
            ),
            jsast::assign_stmt(jsast::id("rh"), jsast::id(i64_pair::HELPER_HI)),
            jsast::assign_stmt(
                jsast::id("ql"),
                jsast::call(
                    jsast::id(i64_pair::HELPER_SDIV),
                    vec![
                        jsast::id("al"),
                        jsast::id("ah"),
                        jsast::id("bl"),
                        jsast::id("bh"),
                    ],
                ),
            ),
            jsast::assign_stmt(jsast::id("qh"), jsast::id(i64_pair::HELPER_HI)),
            jsast::assign_stmt(
                jsast::id("uql"),
                jsast::call(
                    jsast::id(i64_pair::HELPER_UDIV),
                    vec![
                        jsast::id("al"),
                        jsast::id("ah"),
                        jsast::id("bl"),
                        jsast::id("bh"),
                    ],
                ),
            ),
            jsast::assign_stmt(jsast::id("uqh"), jsast::id(i64_pair::HELPER_HI)),
            jsast::assign_stmt(
                jsast::id("url"),
                jsast::call(
                    jsast::id(i64_pair::HELPER_UREM),
                    vec![
                        jsast::id("al"),
                        jsast::id("ah"),
                        jsast::id("bl"),
                        jsast::id("bh"),
                    ],
                ),
            ),
            jsast::assign_stmt(jsast::id("urh"), jsast::id(i64_pair::HELPER_HI)),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("rl"),
                jsast::id("rh"),
                jsast::id("ql"),
                jsast::id("qh"),
                jsast::id("url"),
                jsast::id("urh"),
                jsast::id("uql"),
                jsast::id("uqh"),
            ]))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        printed.contains(i64_pair::HELPER_SDIVREM) && printed.contains(i64_pair::HELPER_UDIVREM),
        "expected specialized sdivrem & udivrem: {printed}"
    );
    assert!(
        !printed.contains(&format!("{}(", i64_pair::HELPER_SREM))
            && !printed.contains(&format!("{}(", i64_pair::HELPER_SDIV))
            && !printed.contains(&format!("{}(", i64_pair::HELPER_UREM))
            && !printed.contains(&format!("{}(", i64_pair::HELPER_UDIV)),
        "expected all 4 specialized calls fused: {printed}"
    );
}

#[test]
fn fast_paths_dead_lo_f64_sign_bit_and_preserves_live_lo() {
    let sign_bit_test = |hi: &str| {
        Expr::Binary(
            BinOp::StrictNe,
            Box::new(Expr::Binary(
                BinOp::BitAnd,
                Box::new(jsast::id(hi)),
                Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
            )),
            Box::new(Expr::Num(0.0)),
        )
    };
    // Dead `lo` sign-bit test: fast-path `x === x ? (x < 0 || x === 0 && 1 / x < 0) : ...`
    // for general floats, and completely eliminates `f64_to_bits` when `x` is known non-NaN.
    let mut decl = Stmt::FunctionDecl {
        name: "is_neg".to_owned(),
        params: vec!["x".to_owned(), "y".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("lo".to_owned(), None),
                ("hi".to_owned(), None),
                ("b".to_owned(), None),
            ]),
            Stmt::If(
                Expr::Binary(
                    BinOp::StrictEq,
                    Box::new(jsast::id("x")),
                    Box::new(jsast::id("y")),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("lo"),
                        jsast::call(jsast::id("$__mires_i64_f64_to_bits"), vec![jsast::id("x")]),
                    ),
                    jsast::assign_stmt(jsast::id("hi"), jsast::id("$__mires_i64_hi")),
                    jsast::assign_stmt(jsast::id("b"), sign_bit_test("hi")),
                    Stmt::Return(Some(jsast::id("b"))),
                ],
                None,
            ),
            Stmt::Return(Some(Expr::Bool(false))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        !printed.contains("$__mires_i64_f64_to_bits"),
        "expected f64_to_bits eliminated when known non-NaN: {printed}"
    );
    assert!(
        printed.contains("1 / x < 0"),
        "expected IEEE-754 negative-zero check: {printed}"
    );

    // Live `lo` must keep `$__mires_i64_f64_to_bits` and never rewrite.
    let mut live_lo = Stmt::FunctionDecl {
        name: "keep_bits".to_owned(),
        params: vec!["x".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("lo".to_owned(), None),
                ("hi".to_owned(), None),
                ("b".to_owned(), None),
            ]),
            jsast::assign_stmt(
                jsast::id("lo"),
                jsast::call(jsast::id("$__mires_i64_f64_to_bits"), vec![jsast::id("x")]),
            ),
            jsast::assign_stmt(jsast::id("hi"), jsast::id("$__mires_i64_hi")),
            jsast::assign_stmt(jsast::id("b"), sign_bit_test("hi")),
            Stmt::Return(Some(Expr::Binary(
                BinOp::BitXor,
                Box::new(jsast::id("lo")),
                Box::new(jsast::id("b")),
            ))),
        ],
    };
    run_speed_post_opts(&mut live_lo);
    let live_printed = jsast::stmt_to_string(&live_lo);
    assert!(
        live_printed.contains("$__mires_i64_f64_to_bits") && !live_printed.contains("1 / x < 0"),
        "live `lo` must preserve f64_to_bits: {live_printed}"
    );
}

#[test]
fn cses_dominating_char_code_at_and_folds_bool_bit_identities() {
    let char_at = || jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]);
    let mut decl = Stmt::FunctionDecl {
        name: "scan".to_owned(),
        params: vec!["s".to_owned(), "i".to_owned(), "x".to_owned()],
        body: vec![
            jsast::lets(vec![("a".to_owned(), None), ("b".to_owned(), None)]),
            jsast::assign_stmt(
                jsast::id("a"),
                Expr::Cond(
                    Box::new(Expr::Binary(
                        BinOp::StrictEq,
                        Box::new(char_at()),
                        Box::new(Expr::Num(32.0)),
                    )),
                    Box::new(Expr::Num(1.0)),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            Stmt::If(
                Expr::Binary(
                    BinOp::StrictNe,
                    Box::new(Expr::Binary(
                        BinOp::StrictEq,
                        Box::new(char_at()),
                        Box::new(Expr::Num(9.0)),
                    )),
                    Box::new(Expr::Bool(false)),
                ),
                vec![jsast::assign_stmt(
                    jsast::id("b"),
                    jsast::imul(
                        Expr::Binary(
                            BinOp::BitAnd,
                            Box::new(Expr::Binary(
                                BinOp::BitAnd,
                                Box::new(jsast::id("x")),
                                Box::new(Expr::Num(255.0)),
                            )),
                            Box::new(Expr::Num(15.0)),
                        ),
                        Expr::Num(1.0),
                    ),
                )],
                None,
            ),
            Stmt::Return(Some(Expr::Binary(
                BinOp::Add,
                Box::new(jsast::id("a")),
                Box::new(jsast::id("b")),
            ))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert_eq!(
        printed.matches("charCodeAt").count(),
        1,
        "expected dominating charCodeAt CSEd to 1 call: {printed}"
    );
    assert!(
        !printed.contains("Math.imul")
            && !printed.contains("? 1 : 0")
            && !printed.contains("!== false"),
        "expected imul(x,1), bool?1:0, and !==false folded: {printed}"
    );
    assert!(
        printed.contains("x & 15"),
        "expected nested bitmask folded to `x & 15`: {printed}"
    );
}

#[test]
fn folds_proven_nonneg_ushr_zero_and_safe_zero_add() {
    let ushr0 = |e: Expr| Expr::Binary(BinOp::UShr, Box::new(e), Box::new(Expr::Num(0.0)));
    let mut decl = Stmt::FunctionDecl {
        name: "alg".to_owned(),
        params: vec![
            "a".to_owned(),
            "b".to_owned(),
            "arr".to_owned(),
            "i".to_owned(),
            "s".to_owned(),
        ],
        body: vec![
            jsast::lets(vec![
                ("m1".to_owned(), None),
                ("m_neg".to_owned(), None),
                ("sh".to_owned(), None),
                ("xr".to_owned(), None),
                ("xr_neg".to_owned(), None),
                ("idx".to_owned(), None),
                ("z_int".to_owned(), None),
                ("z_str".to_owned(), None),
            ]),
            // `(a & 255) >>> 0` -> `a & 255`
            jsast::assign_stmt(
                jsast::id("m1"),
                ushr0(Expr::Binary(
                    BinOp::BitAnd,
                    Box::new(jsast::id("a")),
                    Box::new(Expr::Num(255.0)),
                )),
            ),
            // `(a & 0x80000000) >>> 0` MUST keep `>>> 0` (bit 31 set!)
            jsast::assign_stmt(
                jsast::id("m_neg"),
                ushr0(Expr::Binary(
                    BinOp::BitAnd,
                    Box::new(jsast::id("a")),
                    Box::new(Expr::Num(2147483648.0)),
                )),
            ),
            // `(a >>> 5) >>> 0` -> `a >>> 5`
            jsast::assign_stmt(
                jsast::id("sh"),
                ushr0(Expr::Binary(
                    BinOp::UShr,
                    Box::new(jsast::id("a")),
                    Box::new(Expr::Num(5.0)),
                )),
            ),
            // `((a & 255) ^ (b >>> 24)) >>> 0` -> `(a & 255) ^ (b >>> 24)`
            jsast::assign_stmt(
                jsast::id("xr"),
                ushr0(Expr::Binary(
                    BinOp::BitXor,
                    Box::new(Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("a")),
                        Box::new(Expr::Num(255.0)),
                    )),
                    Box::new(Expr::Binary(
                        BinOp::UShr,
                        Box::new(jsast::id("b")),
                        Box::new(Expr::Num(24.0)),
                    )),
                )),
            ),
            // `((a & 255) ^ (b >>> 0)) >>> 0` MUST keep `>>> 0` (b >>> 0 can have bit 31 set!)
            jsast::assign_stmt(
                jsast::id("xr_neg"),
                ushr0(Expr::Binary(
                    BinOp::BitXor,
                    Box::new(Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("a")),
                        Box::new(Expr::Num(255.0)),
                    )),
                    Box::new(ushr0(jsast::id("b"))),
                )),
            ),
            // `arr[0 + i]` -> `arr[i]`
            jsast::assign_stmt(
                jsast::id("idx"),
                jsast::index(
                    jsast::id("arr"),
                    Expr::Binary(
                        BinOp::Add,
                        Box::new(Expr::Num(0.0)),
                        Box::new(jsast::id("i")),
                    ),
                ),
            ),
            // `0 + (a & 31)` -> `a & 31`
            jsast::assign_stmt(
                jsast::id("z_int"),
                Expr::Binary(
                    BinOp::Add,
                    Box::new(Expr::Num(0.0)),
                    Box::new(Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("a")),
                        Box::new(Expr::Num(31.0)),
                    )),
                ),
            ),
            // `0 + s` MUST NOT fold (could be string concatenation!)
            jsast::assign_stmt(
                jsast::id("z_str"),
                Expr::Binary(
                    BinOp::Add,
                    Box::new(Expr::Num(0.0)),
                    Box::new(jsast::id("s")),
                ),
            ),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("m1"),
                jsast::id("m_neg"),
                jsast::id("sh"),
                jsast::id("xr"),
                jsast::id("xr_neg"),
                jsast::id("idx"),
                jsast::id("z_int"),
                jsast::id("z_str"),
            ]))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        printed.contains("m1 = a & 255;"),
        "expected (a & 255) >>> 0 folded: {printed}"
    );
    assert!(
        printed.contains("m_neg = (a & 2147483648) >>> 0;"),
        "expected (a & 0x80000000) >>> 0 preserved: {printed}"
    );
    assert!(
        printed.contains("sh = a >>> 5;"),
        "expected (a >>> 5) >>> 0 folded: {printed}"
    );
    assert!(
        printed.contains("xr = a & 255 ^ b >>> 24;"),
        "expected proven-nonneg xor >>> 0 folded: {printed}"
    );
    assert!(
        printed.contains("xr_neg = (a & 255 ^ b >>> 0) >>> 0;"),
        "expected possibly-negative xor >>> 0 preserved: {printed}"
    );
    assert!(
        printed.contains("idx = arr[i];"),
        "expected arr[0 + i] folded to arr[i]: {printed}"
    );
    assert!(
        printed.contains("z_int = a & 31;"),
        "expected 0 + (a & 31) folded: {printed}"
    );
    assert!(
        printed.contains("z_str = 0 + s;"),
        "0 + arbitrary ident must NOT fold: {printed}"
    );
}

#[test]
fn folds_ieee_max_min_two_var_and_const_idioms_and_rejects_unsound_neg_zero() {
    let sign_neg = |v: &str| {
        Expr::Binary(
            BinOp::Or,
            Box::new(Expr::Binary(
                BinOp::Lt,
                Box::new(jsast::id(v)),
                Box::new(Expr::Num(0.0)),
            )),
            Box::new(Expr::Binary(
                BinOp::And,
                Box::new(Expr::Binary(
                    BinOp::StrictEq,
                    Box::new(jsast::id(v)),
                    Box::new(Expr::Num(0.0)),
                )),
                Box::new(Expr::Binary(
                    BinOp::Lt,
                    Box::new(Expr::Binary(
                        BinOp::Div,
                        Box::new(Expr::Num(1.0)),
                        Box::new(jsast::id(v)),
                    )),
                    Box::new(Expr::Num(0.0)),
                )),
            )),
        )
    };
    let assign_brk = |dst: &str, val: Expr, lbl: &str| {
        vec![
            jsast::assign_stmt(jsast::id(dst), val),
            Stmt::Break(Some(lbl.to_owned())),
        ]
    };

    // 1. Exact 2-variable `Math.max(k, r)` + 2-variable `Math.min(k, r)` + 1-var const `1` + `+0.0`.
    let mut decl = Stmt::FunctionDecl {
        name: "minmax".to_owned(),
        params: vec!["k".to_owned(), "r".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("c_".to_owned(), None),
                ("da".to_owned(), None),
                ("dd".to_owned(), None),
                ("j".to_owned(), None),
                ("c2".to_owned(), None),
                ("da2".to_owned(), None),
                ("dd2".to_owned(), None),
                ("m".to_owned(), None),
                ("dh".to_owned(), None),
                ("y1".to_owned(), None),
                ("d0".to_owned(), None),
                ("y0max".to_owned(), None),
                ("d0m".to_owned(), None),
                ("y0min".to_owned(), None),
            ]),
            // Math.max(k, r)
            jsast::assign_stmt(
                jsast::id("c_"),
                Expr::Binary(
                    BinOp::StrictNe,
                    Box::new(jsast::id("k")),
                    Box::new(jsast::id("k")),
                ),
            ),
            Stmt::Labeled(
                "c".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        Expr::Unary(UnOp::Not, Box::new(jsast::id("c_"))),
                        vec![
                            jsast::assign_stmt(
                                jsast::id("da"),
                                Expr::Binary(
                                    BinOp::StrictNe,
                                    Box::new(jsast::id("r")),
                                    Box::new(jsast::id("r")),
                                ),
                            ),
                            Stmt::If(
                                Expr::Unary(UnOp::Not, Box::new(jsast::id("da"))),
                                vec![Stmt::If(
                                    Expr::Binary(
                                        BinOp::StrictEq,
                                        Box::new(jsast::id("k")),
                                        Box::new(jsast::id("r")),
                                    ),
                                    vec![
                                        jsast::assign_stmt(jsast::id("dd"), sign_neg("k")),
                                        Stmt::If(
                                            jsast::id("dd"),
                                            assign_brk("j", jsast::id("r"), "c"),
                                            Some(assign_brk("j", jsast::id("k"), "c")),
                                        ),
                                    ],
                                    Some(vec![Stmt::If(
                                        Expr::Binary(
                                            BinOp::Gt,
                                            Box::new(jsast::id("k")),
                                            Box::new(jsast::id("r")),
                                        ),
                                        assign_brk("j", jsast::id("k"), "c"),
                                        Some(assign_brk("j", jsast::id("r"), "c")),
                                    )]),
                                )],
                                None,
                            ),
                        ],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("j"), jsast::id("NaN")),
                ])),
            ),
            // Math.min(k, r)
            jsast::assign_stmt(
                jsast::id("c2"),
                Expr::Binary(
                    BinOp::StrictNe,
                    Box::new(jsast::id("k")),
                    Box::new(jsast::id("k")),
                ),
            ),
            Stmt::Labeled(
                "c2l".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        Expr::Unary(UnOp::Not, Box::new(jsast::id("c2"))),
                        vec![
                            jsast::assign_stmt(
                                jsast::id("da2"),
                                Expr::Binary(
                                    BinOp::StrictNe,
                                    Box::new(jsast::id("r")),
                                    Box::new(jsast::id("r")),
                                ),
                            ),
                            Stmt::If(
                                Expr::Unary(UnOp::Not, Box::new(jsast::id("da2"))),
                                vec![Stmt::If(
                                    Expr::Binary(
                                        BinOp::StrictEq,
                                        Box::new(jsast::id("k")),
                                        Box::new(jsast::id("r")),
                                    ),
                                    vec![
                                        jsast::assign_stmt(jsast::id("dd2"), sign_neg("k")),
                                        Stmt::If(
                                            jsast::id("dd2"),
                                            assign_brk("m", jsast::id("k"), "c2l"),
                                            Some(assign_brk("m", jsast::id("r"), "c2l")),
                                        ),
                                    ],
                                    Some(vec![Stmt::If(
                                        Expr::Binary(
                                            BinOp::Lt,
                                            Box::new(jsast::id("k")),
                                            Box::new(jsast::id("r")),
                                        ),
                                        assign_brk("m", jsast::id("k"), "c2l"),
                                        Some(assign_brk("m", jsast::id("r"), "c2l")),
                                    )]),
                                )],
                                None,
                            ),
                        ],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("m"), jsast::id("NaN")),
                ])),
            ),
            // Constant-1 variant: `Math.max(j, 1)`
            jsast::assign_stmt(
                jsast::id("dh"),
                Expr::Binary(
                    BinOp::StrictNe,
                    Box::new(jsast::id("j")),
                    Box::new(jsast::id("j")),
                ),
            ),
            Stmt::Labeled(
                "f".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        Expr::Unary(UnOp::Not, Box::new(jsast::id("dh"))),
                        vec![Stmt::If(
                            Expr::Binary(
                                BinOp::StrictEq,
                                Box::new(jsast::id("j")),
                                Box::new(Expr::Num(1.0)),
                            ),
                            assign_brk("y1", jsast::id("j"), "f"),
                            Some(vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::Gt,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(1.0)),
                                ),
                                assign_brk("y1", jsast::id("j"), "f"),
                                Some(assign_brk("y1", Expr::Num(1.0), "f")),
                            )]),
                        )],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("y1"), jsast::id("NaN")),
                ])),
            ),
            // Constant +0.0 MAX (picks `+0.0` on `j === 0`) -> valid `Math.max(j, 0)`
            jsast::assign_stmt(
                jsast::id("d0"),
                Expr::Binary(
                    BinOp::StrictNe,
                    Box::new(jsast::id("j")),
                    Box::new(jsast::id("j")),
                ),
            ),
            Stmt::Labeled(
                "f0".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        Expr::Unary(UnOp::Not, Box::new(jsast::id("d0"))),
                        vec![Stmt::If(
                            Expr::Binary(
                                BinOp::StrictEq,
                                Box::new(jsast::id("j")),
                                Box::new(Expr::Num(0.0)),
                            ),
                            assign_brk("y0max", Expr::Num(0.0), "f0"),
                            Some(vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::Gt,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(0.0)),
                                ),
                                assign_brk("y0max", jsast::id("j"), "f0"),
                                Some(assign_brk("y0max", Expr::Num(0.0), "f0")),
                            )]),
                        )],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("y0max"), jsast::id("NaN")),
                ])),
            ),
            // Constant +0.0 MIN (picks `j` on `j === 0`, preserving `-0.0`) -> valid `Math.min(j, 0)`
            jsast::assign_stmt(
                jsast::id("d0m"),
                Expr::Binary(
                    BinOp::StrictNe,
                    Box::new(jsast::id("j")),
                    Box::new(jsast::id("j")),
                ),
            ),
            Stmt::Labeled(
                "f0m".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        Expr::Unary(UnOp::Not, Box::new(jsast::id("d0m"))),
                        vec![Stmt::If(
                            Expr::Binary(
                                BinOp::StrictEq,
                                Box::new(jsast::id("j")),
                                Box::new(Expr::Num(0.0)),
                            ),
                            assign_brk("y0min", jsast::id("j"), "f0m"),
                            Some(vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::Lt,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(0.0)),
                                ),
                                assign_brk("y0min", jsast::id("j"), "f0m"),
                                Some(assign_brk("y0min", Expr::Num(0.0), "f0m")),
                            )]),
                        )],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("y0min"), jsast::id("NaN")),
                ])),
            ),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("j"),
                jsast::id("m"),
                jsast::id("y1"),
                jsast::id("y0max"),
                jsast::id("y0min"),
            ]))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        printed.contains("j = Math.max(k, r);"),
        "expected 2-var Math.max folded: {printed}"
    );
    assert!(
        printed.contains("m = Math.min(k, r);"),
        "expected 2-var Math.min folded: {printed}"
    );
    assert!(
        printed.contains("y1 = Math.max(j, 1);"),
        "expected const-1 Math.max folded: {printed}"
    );
    assert!(
        printed.contains("y0max = Math.max(j, 0);"),
        "expected const-+0 Math.max folded: {printed}"
    );
    assert!(
        printed.contains("y0min = Math.min(j, 0);"),
        "expected const-+0 Math.min folded: {printed}"
    );
    assert!(
        !printed.contains("NaN") && !printed.contains("break "),
        "all five labeled NaN blocks should collapse cleanly: {printed}"
    );

    // 2. Negative `-0.0` soundness: MAX against `+0.0` that returns `j` on `j === 0` (which
    //    would return `-0.0` instead of `+0.0`) MUST NOT fold to `Math.max(j, 0)`.
    let mut bad_neg0 = Stmt::FunctionDecl {
        name: "bad_zero".to_owned(),
        params: vec!["j".to_owned()],
        body: vec![
            jsast::lets(vec![("dh".to_owned(), None), ("y".to_owned(), None)]),
            jsast::assign_stmt(
                jsast::id("dh"),
                Expr::Binary(
                    BinOp::StrictNe,
                    Box::new(jsast::id("j")),
                    Box::new(jsast::id("j")),
                ),
            ),
            Stmt::Labeled(
                "f".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        Expr::Unary(UnOp::Not, Box::new(jsast::id("dh"))),
                        vec![Stmt::If(
                            Expr::Binary(
                                BinOp::StrictEq,
                                Box::new(jsast::id("j")),
                                Box::new(Expr::Num(0.0)),
                            ),
                            assign_brk("y", jsast::id("j"), "f"),
                            Some(vec![Stmt::If(
                                Expr::Binary(
                                    BinOp::Gt,
                                    Box::new(jsast::id("j")),
                                    Box::new(Expr::Num(0.0)),
                                ),
                                assign_brk("y", jsast::id("j"), "f"),
                                Some(assign_brk("y", Expr::Num(0.0), "f")),
                            )]),
                        )],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("y"), jsast::id("NaN")),
                ])),
            ),
            Stmt::Return(Some(jsast::id("y"))),
        ],
    };
    run_speed_post_opts(&mut bad_neg0);
    let sb = jsast::stmt_to_string(&bad_neg0);
    assert!(
        !sb.contains("Math.max") && sb.contains("NaN"),
        "wrong -0.0 tie-break on MAX against +0.0 must NOT fold to Math.max: {sb}"
    );
}

#[test]
fn folds_unsigned_range_checks_and_rejects_unsound_floats_and_nan() {
    // 1. Positive cases: bitwise masked int, UShr strict bounds, negated `||`, lower bound 0,
    //    in-bounds `charCodeAt` in counted loop, single-read temp `if/else`, unbounded
    //    `charCodeAt` with `A >= 1` (`| 0` guarded), and `charCodeAt` promoted by `< 128`.
    let mut pos = Stmt::FunctionDecl {
        name: "ranges_ok".to_owned(),
        params: vec!["a".to_owned(), "s".to_owned(), "oob".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("x".to_owned(), None),
                ("y".to_owned(), None),
                ("r1".to_owned(), None),
                ("r2".to_owned(), None),
                ("r3".to_owned(), None),
                ("r4".to_owned(), None),
                ("r5".to_owned(), None),
                ("k".to_owned(), None),
                ("m".to_owned(), None),
                ("c".to_owned(), None),
                ("ae".to_owned(), None),
                ("r_loop".to_owned(), None),
                ("u".to_owned(), None),
                ("r_oob_and".to_owned(), None),
                ("r_prom".to_owned(), None),
            ]),
            // x = a & 255 (NonNegI31)
            jsast::assign_stmt(
                jsast::id("x"),
                jsast::binary(BinOp::BitAnd, jsast::id("a"), Expr::Num(255.0)),
            ),
            // y = a >>> 1 (NonNegI31)
            jsast::assign_stmt(
                jsast::id("y"),
                jsast::binary(BinOp::UShr, jsast::id("a"), Expr::Num(1.0)),
            ),
            // r1 = x >= 65 && x <= 90 -> x - 65 >>> 0 <= 25
            jsast::assign_stmt(
                jsast::id("r1"),
                jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::Ge, jsast::id("x"), Expr::Num(65.0)),
                    jsast::binary(BinOp::Le, jsast::id("x"), Expr::Num(90.0)),
                ),
            ),
            // r2 = 97 <= x && 122 >= x -> x - 97 >>> 0 <= 25
            jsast::assign_stmt(
                jsast::id("r2"),
                jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::Le, Expr::Num(97.0), jsast::id("x")),
                    jsast::binary(BinOp::Ge, Expr::Num(122.0), jsast::id("x")),
                ),
            ),
            // r3 = y > 47 && y < 58 -> y - 48 >>> 0 <= 9
            jsast::assign_stmt(
                jsast::id("r3"),
                jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::Gt, jsast::id("y"), Expr::Num(47.0)),
                    jsast::binary(BinOp::Lt, jsast::id("y"), Expr::Num(58.0)),
                ),
            ),
            // r4 = x < 65 || x > 90 -> x - 65 >>> 0 > 25
            jsast::assign_stmt(
                jsast::id("r4"),
                jsast::binary(
                    BinOp::Or,
                    jsast::binary(BinOp::Lt, jsast::id("x"), Expr::Num(65.0)),
                    jsast::binary(BinOp::Gt, jsast::id("x"), Expr::Num(90.0)),
                ),
            ),
            // r5 = x >= 0 && x <= 127 -> x <= 127
            jsast::assign_stmt(
                jsast::id("r5"),
                jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::Ge, jsast::id("x"), Expr::Num(0.0)),
                    jsast::binary(BinOp::Le, jsast::id("x"), Expr::Num(127.0)),
                ),
            ),
            // In-bounds counted loop `charCodeAt` + single-read temp `if/else` (`jmespath` idiom)
            jsast::assign_stmt(jsast::id("k"), jsast::member(jsast::id("s"), "length")),
            jsast::assign_stmt(jsast::id("m"), Expr::Num(0.0)),
            Stmt::LoopForever(
                None,
                vec![Stmt::If(
                    jsast::binary(BinOp::Lt, jsast::id("m"), jsast::id("k")),
                    vec![
                        jsast::assign_stmt(
                            jsast::id("c"),
                            jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("m")]),
                        ),
                        jsast::assign_stmt(
                            jsast::id("ae"),
                            jsast::binary(BinOp::Le, Expr::Num(65.0), jsast::id("c")),
                        ),
                        Stmt::If(
                            jsast::id("ae"),
                            vec![jsast::assign_stmt(
                                jsast::id("r_loop"),
                                jsast::binary(BinOp::Le, jsast::id("c"), Expr::Num(90.0)),
                            )],
                            Some(vec![jsast::assign_stmt(
                                jsast::id("r_loop"),
                                Expr::Bool(false),
                            )]),
                        ),
                        jsast::assign_stmt(
                            jsast::id("m"),
                            jsast::binary(
                                BinOp::UShr,
                                jsast::binary(BinOp::Add, jsast::id("m"), Expr::Num(1.0)),
                                Expr::Num(0.0),
                            ),
                        ),
                    ],
                    Some(vec![Stmt::Break(None)]),
                )],
            ),
            // Unbounded `u = s.charCodeAt(oob)`: `48 <= u && u <= 57` uses `(u | 0) - 48 >>> 0 <= 9`,
            // and after `if (u < 128)` `u` is promoted non-NaN -> `u - 9 >>> 0 <= 4`.
            jsast::assign_stmt(
                jsast::id("u"),
                jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("oob")]),
            ),
            Stmt::If(
                jsast::binary(BinOp::Le, Expr::Num(48.0), jsast::id("u")),
                vec![jsast::assign_stmt(
                    jsast::id("r_oob_and"),
                    jsast::binary(BinOp::Le, jsast::id("u"), Expr::Num(57.0)),
                )],
                Some(vec![jsast::assign_stmt(
                    jsast::id("r_oob_and"),
                    Expr::Bool(false),
                )]),
            ),
            Stmt::If(
                jsast::binary(BinOp::Lt, jsast::id("u"), Expr::Num(128.0)),
                vec![jsast::assign_stmt(
                    jsast::id("r_prom"),
                    jsast::binary(
                        BinOp::And,
                        jsast::binary(BinOp::Ge, jsast::id("u"), Expr::Num(9.0)),
                        jsast::binary(BinOp::Le, jsast::id("u"), Expr::Num(13.0)),
                    ),
                )],
                None,
            ),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("r1"),
                jsast::id("r2"),
                jsast::id("r3"),
                jsast::id("r4"),
                jsast::id("r5"),
                jsast::id("r_loop"),
                jsast::id("r_oob_and"),
                jsast::id("r_prom"),
            ]))),
        ],
    };
    run_speed_post_opts(&mut pos);
    let p = jsast::stmt_to_string(&pos);
    assert!(
        p.contains("r1 = (x - 65 >>> 0) <= 25;") || p.contains("r1 = x - 65 >>> 0 <= 25;"),
        "r1: {p}"
    );
    assert!(
        p.contains("r2 = (x - 97 >>> 0) <= 25;") || p.contains("r2 = x - 97 >>> 0 <= 25;"),
        "r2: {p}"
    );
    assert!(
        p.contains("r3 = (y - 48 >>> 0) <= 9;") || p.contains("r3 = y - 48 >>> 0 <= 9;"),
        "r3: {p}"
    );
    assert!(
        p.contains("r4 = (x - 65 >>> 0) > 25;") || p.contains("r4 = x - 65 >>> 0 > 25;"),
        "r4: {p}"
    );
    assert!(p.contains("r5 = x <= 127;"), "r5: {p}");
    assert!(
        (p.contains("r_loop = c - 65 >>> 0 <= 25;")
            || p.contains("r_loop = (c - 65 >>> 0) <= 25;"))
            && !p.contains("ae ="),
        "in-bounds charCodeAt should fold without |0 and drop dead ae: {p}"
    );
    assert!(
        p.contains("r_oob_and = (u | 0) - 48 >>> 0 <= 9;")
            || p.contains("r_oob_and = ((u | 0) - 48 >>> 0) <= 9;"),
        "unbounded charCodeAt && must guard NaN via (u | 0): {p}"
    );
    assert!(
        p.contains("r_prom = u - 9 >>> 0 <= 4;") || p.contains("r_prom = (u - 9 >>> 0) <= 4;"),
        "promoted < 128 charCodeAt should fold without |0: {p}"
    );

    // 2. Negative soundness cases: arbitrary param (could be 64.5 / NaN / 2^32+65),
    //    float division result, unbounded charCodeAt in `||`, and unbounded charCodeAt at A=0.
    let mut neg = Stmt::FunctionDecl {
        name: "ranges_reject".to_owned(),
        params: vec!["p".to_owned(), "s".to_owned(), "oob".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("f".to_owned(), None),
                ("u".to_owned(), None),
                ("b1".to_owned(), None),
                ("b2".to_owned(), None),
                ("b3".to_owned(), None),
                ("b4".to_owned(), None),
            ]),
            jsast::assign_stmt(
                jsast::id("f"),
                jsast::binary(BinOp::Div, jsast::id("p"), Expr::Num(2.0)),
            ),
            jsast::assign_stmt(
                jsast::id("u"),
                jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("oob")]),
            ),
            // Arbitrary param `p`
            jsast::assign_stmt(
                jsast::id("b1"),
                jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::Ge, jsast::id("p"), Expr::Num(65.0)),
                    jsast::binary(BinOp::Le, jsast::id("p"), Expr::Num(90.0)),
                ),
            ),
            // Float `f = p / 2`
            jsast::assign_stmt(
                jsast::id("b2"),
                jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::Ge, jsast::id("f"), Expr::Num(1.0)),
                    jsast::binary(BinOp::Le, jsast::id("f"), Expr::Num(10.0)),
                ),
            ),
            // Possibly-NaN `u` in negated `||` (NaN < 65 || NaN > 90 is false!)
            jsast::assign_stmt(
                jsast::id("b3"),
                jsast::binary(
                    BinOp::Or,
                    jsast::binary(BinOp::Lt, jsast::id("u"), Expr::Num(65.0)),
                    jsast::binary(BinOp::Gt, jsast::id("u"), Expr::Num(90.0)),
                ),
            ),
            // Possibly-NaN `u` at A == 0 (NaN >= 0 && NaN <= 9 is false, (NaN|0) <= 9 would be true!)
            jsast::assign_stmt(
                jsast::id("b4"),
                jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::Ge, jsast::id("u"), Expr::Num(0.0)),
                    jsast::binary(BinOp::Le, jsast::id("u"), Expr::Num(9.0)),
                ),
            ),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("b1"),
                jsast::id("b2"),
                jsast::id("b3"),
                jsast::id("b4"),
            ]))),
        ],
    };
    run_speed_post_opts(&mut neg);
    let np = jsast::stmt_to_string(&neg);
    assert!(
        !np.contains(">>> 0"),
        "unsound float/arbitrary/possibly-NaN range checks must NOT fold: {np}"
    );
}

#[test]
fn lowers_dense_ascii_luts_and_rejects_non_numeric_and_sparse() {
    let eq_chain = |var: &str, bytes: &[u8], op: BinOp, cmp: BinOp| -> Expr {
        let mut iter = bytes
            .iter()
            .map(|&b| jsast::binary(cmp, jsast::id(var), Expr::Num(f64::from(b))));
        let first = iter.next().unwrap();
        iter.fold(first, |acc, next| jsast::binary(op, acc, next))
    };

    // 1. Positive: `||` chain, `&&` chain, boolean switch, control-flow switch, and 256-byte switch.
    let mut pos = Stmt::FunctionDecl {
        name: "scan_delim".to_owned(),
        params: vec!["s".to_owned(), "i".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("c".to_owned(), None),
                ("d1".to_owned(), None),
                ("d2".to_owned(), None),
                ("d3".to_owned(), None),
                ("hit".to_owned(), None),
            ]),
            jsast::assign_stmt(
                jsast::id("c"),
                jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]),
            ),
            jsast::assign_stmt(
                jsast::id("d1"),
                eq_chain(
                    "c",
                    &[33, 42, 60, 91, 92, 95, 96, 126],
                    BinOp::Or,
                    BinOp::StrictEq,
                ),
            ),
            jsast::assign_stmt(
                jsast::id("d2"),
                eq_chain("c", &[33, 42, 60, 91, 92, 95], BinOp::And, BinOp::StrictNe),
            ),
            Stmt::Switch(
                jsast::id("c"),
                vec![
                    jsast::SwitchCase {
                        tests: vec![33, 35, 36, 37, 38, 39, 126]
                            .into_iter()
                            .map(|b| Expr::Num(f64::from(b)))
                            .collect(),
                        body: vec![
                            jsast::assign_stmt(jsast::id("d3"), Expr::Bool(true)),
                            Stmt::Break(None),
                        ],
                    },
                    jsast::SwitchCase {
                        tests: Vec::new(),
                        body: vec![jsast::assign_stmt(jsast::id("d3"), Expr::Bool(false))],
                    },
                ],
            ),
            Stmt::Labeled(
                "a".to_owned(),
                Box::new(Stmt::Block(vec![Stmt::Switch(
                    jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]),
                    vec![
                        jsast::SwitchCase {
                            tests: vec![33, 42, 60, 91, 92, 200]
                                .into_iter()
                                .map(|b| Expr::Num(f64::from(b)))
                                .collect(),
                            body: vec![
                                jsast::assign_stmt(jsast::id("hit"), Expr::Num(1.0)),
                                Stmt::Break(Some("a".to_owned())),
                            ],
                        },
                        jsast::SwitchCase {
                            tests: Vec::new(),
                            body: vec![jsast::assign_stmt(jsast::id("hit"), Expr::Num(0.0))],
                        },
                    ],
                )])),
            ),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("d1"),
                jsast::id("d2"),
                jsast::id("d3"),
                jsast::id("hit"),
            ]))),
        ],
    };
    // Pricing trial (`allow_duplicating = false`) must NOT emit 128-byte LUTs.
    let mut trial = pos.clone();
    run_speed_post_opts_with_mode(&mut trial, false, &HashSet::new());
    assert!(
        !jsast::stmt_to_string(&trial).contains("Uint8Array"),
        "pricing trial must not bloat AST with inline Uint8Array LUTs"
    );

    run_speed_post_opts(&mut pos);
    let printed = jsast::stmt_to_string(&pos);
    assert_eq!(
        printed.matches("new Uint8Array(").count(),
        4,
        "expected all 4 dense ASCII tests lowered to Uint8Array LUTs: {printed}"
    );
    assert!(
        !printed.contains("switch"),
        "both dense ASCII switches should be replaced by LUT checks: {printed}"
    );
    assert!(
        printed.contains("!== 1"),
        "negated && byte chain should emit !== 1: {printed}"
    );

    // 2. Negative soundness cases: non-numeric param `p` (could be "33"!), sparse >255 Unicode
    //    switch, < 6 distinct bytes, and switch falling through to default.
    let mut neg = Stmt::FunctionDecl {
        name: "reject_luts".to_owned(),
        params: vec!["p".to_owned(), "s".to_owned(), "i".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("c".to_owned(), None),
                ("bad_str".to_owned(), None),
                ("too_few".to_owned(), None),
                ("uni".to_owned(), None),
                ("ft".to_owned(), None),
            ]),
            jsast::assign_stmt(
                jsast::id("c"),
                jsast::method_call(jsast::id("s"), "charCodeAt", vec![jsast::id("i")]),
            ),
            // Non-numeric param `p`
            jsast::assign_stmt(
                jsast::id("bad_str"),
                eq_chain("p", &[33, 42, 60, 91, 92, 95], BinOp::Or, BinOp::StrictEq),
            ),
            // Only 5 distinct bytes
            jsast::assign_stmt(
                jsast::id("too_few"),
                eq_chain("c", &[33, 42, 60, 91, 92], BinOp::Or, BinOp::StrictEq),
            ),
            // Unicode > 255 switch
            Stmt::Switch(
                jsast::id("c"),
                vec![
                    jsast::SwitchCase {
                        tests: vec![160, 5760, 8232, 8233, 8239, 8287, 12288, 65279]
                            .into_iter()
                            .map(|b| Expr::Num(f64::from(b)))
                            .collect(),
                        body: vec![
                            jsast::assign_stmt(jsast::id("uni"), Expr::Bool(true)),
                            Stmt::Break(None),
                        ],
                    },
                    jsast::SwitchCase {
                        tests: Vec::new(),
                        body: vec![jsast::assign_stmt(jsast::id("uni"), Expr::Bool(false))],
                    },
                ],
            ),
            // Fallthrough from matching arm into default (no `break`)
            Stmt::Switch(
                jsast::id("c"),
                vec![
                    jsast::SwitchCase {
                        tests: vec![33, 35, 36, 37, 38, 39]
                            .into_iter()
                            .map(|b| Expr::Num(f64::from(b)))
                            .collect(),
                        body: vec![jsast::assign_stmt(jsast::id("ft"), Expr::Num(1.0))],
                    },
                    jsast::SwitchCase {
                        tests: Vec::new(),
                        body: vec![jsast::assign_stmt(jsast::id("ft"), Expr::Num(2.0))],
                    },
                ],
            ),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("bad_str"),
                jsast::id("too_few"),
                jsast::id("uni"),
                jsast::id("ft"),
            ]))),
        ],
    };
    run_speed_post_opts(&mut neg);
    let np = jsast::stmt_to_string(&neg);
    assert!(
        !np.contains("Uint8Array"),
        "non-numeric, <6, >255, and fallthrough cases must NOT lower to Uint8Array LUT: {np}"
    );
}

#[test]
fn lowers_chained_string_eq_to_switch_and_preserves_breaks_and_duplicates() {
    let str_eq = |var: &str, s: &str| {
        jsast::binary(BinOp::StrictEq, jsast::id(var), Expr::Str(s.to_owned()))
    };
    let rev_eq = |s: &str, var: &str| {
        jsast::binary(BinOp::StrictEq, Expr::Str(s.to_owned()), jsast::id(var))
    };
    let not_eq = |var: &str, s: &str| Expr::Unary(UnOp::Not, Box::new(str_eq(var, s)));

    // 1. Positive A: Labeled `r: if ("EQ" === k) ... else if ("NE" === k) ... else { if ("LT" === k) { ... break r; } ... }`
    let mut comparator = Stmt::FunctionDecl {
        name: "cmp".to_owned(),
        params: vec!["k".to_owned()],
        body: vec![
            jsast::lets(vec![("e".to_owned(), None)]),
            Stmt::Labeled(
                "r".to_owned(),
                Box::new(Stmt::If(
                    rev_eq("EQ", "k"),
                    vec![jsast::assign_stmt(jsast::id("e"), Expr::Num(1.0))],
                    Some(vec![Stmt::If(
                        rev_eq("NE", "k"),
                        vec![jsast::assign_stmt(jsast::id("e"), Expr::Num(2.0))],
                        Some(vec![
                            Stmt::If(
                                rev_eq("LT", "k"),
                                vec![
                                    jsast::assign_stmt(jsast::id("e"), Expr::Num(3.0)),
                                    Stmt::Break(Some("r".to_owned())),
                                ],
                                None,
                            ),
                            Stmt::If(
                                rev_eq("GT", "k"),
                                vec![
                                    jsast::assign_stmt(jsast::id("e"), Expr::Num(4.0)),
                                    Stmt::Break(Some("r".to_owned())),
                                ],
                                None,
                            ),
                            jsast::assign_stmt(jsast::id("e"), Expr::Num(5.0)),
                        ]),
                    )]),
                )),
            ),
            Stmt::Return(Some(jsast::id("e"))),
        ],
    };
    run_speed_post_opts(&mut comparator);
    let cp = jsast::stmt_to_string(&comparator);
    assert!(
        cp.contains("r: switch (k)")
            && cp.contains("case \"EQ\":")
            && cp.contains("case \"NE\":")
            && cp.contains("case \"LT\":")
            && cp.contains("case \"GT\":")
            && cp.contains("default:")
            && !cp.contains("else if"),
        "expected labeled comparator chain lowered to r: switch (k): {cp}"
    );

    // 2. Positive B: Negated multi-key `jmespath$functions::call` tree -> ONE flat `switch (a)`.
    let mut multi_neg = Stmt::FunctionDecl {
        name: "dispatch_fn".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            jsast::lets(vec![("f".to_owned(), None)]),
            Stmt::Labeled(
                "b".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        jsast::binary(BinOp::And, not_eq("a", "abs"), not_eq("a", "floor")),
                        vec![Stmt::If(
                            str_eq("a", "contains"),
                            vec![
                                jsast::assign_stmt(jsast::id("f"), Expr::Num(2.0)),
                                Stmt::Break(Some("b".to_owned())),
                            ],
                            Some(vec![
                                Stmt::If(
                                    jsast::binary(
                                        BinOp::And,
                                        not_eq("a", "starts_with"),
                                        not_eq("a", "ends_with"),
                                    ),
                                    vec![Stmt::If(
                                        str_eq("a", "length"),
                                        vec![
                                            jsast::assign_stmt(jsast::id("f"), Expr::Num(4.0)),
                                            Stmt::Break(Some("b".to_owned())),
                                        ],
                                        Some(vec![
                                            jsast::assign_stmt(jsast::id("f"), Expr::Num(0.0)),
                                            Stmt::Break(Some("b".to_owned())),
                                        ]),
                                    )],
                                    None,
                                ),
                                jsast::assign_stmt(jsast::id("f"), Expr::Num(3.0)),
                                Stmt::Break(Some("b".to_owned())),
                            ]),
                        )],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("f"), Expr::Num(1.0)),
                ])),
            ),
            Stmt::Return(Some(jsast::id("f"))),
        ],
    };
    run_speed_post_opts(&mut multi_neg);
    let mp = jsast::stmt_to_string(&multi_neg);
    assert!(
        mp.contains("switch (a)")
            && mp.contains("case \"abs\":")
            && mp.contains("case \"floor\":")
            && mp.contains("case \"contains\":")
            && mp.contains("case \"starts_with\":")
            && mp.contains("case \"ends_with\":")
            && mp.contains("case \"length\":")
            && mp.contains("default:")
            && !mp.contains("if ("),
        "expected negated multi-key dispatch tree collapsed into flat switch: {mp}"
    );

    // 2b. Positive C: Negated multi-key chain whose `then` exits via an exhaustive labeled-break
    //     `switch (tag)` (`jmespath$eval$eval` idiom for `Field`/`Identity`/`Literal`/`Pipe`/`Index`).
    let mut switch_exit = Stmt::FunctionDecl {
        name: "eval_like".to_owned(),
        params: vec!["e".to_owned(), "tag".to_owned()],
        body: vec![
            jsast::lets(vec![("c".to_owned(), None)]),
            Stmt::Labeled(
                "b".to_owned(),
                Box::new(Stmt::Block(vec![Stmt::If(
                    str_eq("e", "Field"),
                    vec![jsast::assign_stmt(jsast::id("c"), Expr::Num(1.0))],
                    Some(vec![
                        Stmt::If(
                            jsast::binary(
                                BinOp::And,
                                not_eq("e", "Identity"),
                                not_eq("e", "Current"),
                            ),
                            vec![Stmt::If(
                                str_eq("e", "Literal"),
                                vec![
                                    jsast::assign_stmt(jsast::id("c"), Expr::Num(3.0)),
                                    Stmt::Break(Some("b".to_owned())),
                                ],
                                Some(vec![
                                    Stmt::If(
                                        not_eq("e", "Pipe"),
                                        vec![
                                            jsast::assign_stmt(jsast::id("c"), Expr::Num(5.0)),
                                            Stmt::Break(Some("b".to_owned())),
                                        ],
                                        None,
                                    ),
                                    Stmt::Switch(
                                        jsast::id("tag"),
                                        vec![
                                            jsast::SwitchCase {
                                                tests: vec![Expr::Num(0.0)],
                                                body: vec![
                                                    jsast::assign_stmt(
                                                        jsast::id("c"),
                                                        Expr::Num(4.0),
                                                    ),
                                                    Stmt::Break(Some("b".to_owned())),
                                                ],
                                            },
                                            jsast::SwitchCase {
                                                tests: Vec::new(),
                                                body: vec![Stmt::Throw(Expr::Str(
                                                    "err".to_owned(),
                                                ))],
                                            },
                                        ],
                                    ),
                                ]),
                            )],
                            None,
                        ),
                        jsast::assign_stmt(jsast::id("c"), Expr::Num(2.0)),
                    ]),
                )])),
            ),
            Stmt::Return(Some(jsast::id("c"))),
        ],
    };
    run_speed_post_opts(&mut switch_exit);
    let sep = jsast::stmt_to_string(&switch_exit);
    assert!(
        sep.contains("switch (e)")
            && sep.contains("case \"Field\":")
            && sep.contains("case \"Identity\":")
            && sep.contains("case \"Current\":")
            && sep.contains("case \"Literal\":")
            && sep.contains("case \"Pipe\":")
            && !sep.contains("if ("),
        "exhaustive non-breaking inner switch should allow full outer string switch collapse: {sep}"
    );

    // 3. Negative A: Unlabeled `break;` inside a loop MUST NOT convert to `switch`!
    let mut loop_brk = Stmt::FunctionDecl {
        name: "in_loop".to_owned(),
        params: vec!["k".to_owned()],
        body: vec![
            jsast::lets(vec![("r".to_owned(), Some(Expr::Num(0.0)))]),
            Stmt::LoopForever(
                None,
                vec![Stmt::If(
                    str_eq("k", "A"),
                    vec![
                        jsast::assign_stmt(jsast::id("r"), Expr::Num(1.0)),
                        Stmt::Break(None),
                    ],
                    Some(vec![Stmt::If(
                        str_eq("k", "B"),
                        vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(2.0))],
                        Some(vec![Stmt::If(
                            str_eq("k", "C"),
                            vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(3.0))],
                            Some(vec![Stmt::If(
                                str_eq("k", "D"),
                                vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(4.0))],
                                None,
                            )]),
                        )]),
                    )]),
                )],
            ),
            Stmt::Return(Some(jsast::id("r"))),
        ],
    };
    run_speed_post_opts(&mut loop_brk);
    assert!(
        !jsast::stmt_to_string(&loop_brk).contains("switch"),
        "unlabeled break inside loop must NEVER be captured by synthesized switch"
    );

    // 4. Negative B: Duplicate string key MUST NOT convert to `switch`!
    let mut dup_key = Stmt::FunctionDecl {
        name: "dup".to_owned(),
        params: vec!["k".to_owned()],
        body: vec![
            jsast::lets(vec![("r".to_owned(), None)]),
            Stmt::If(
                str_eq("k", "A"),
                vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(1.0))],
                Some(vec![Stmt::If(
                    str_eq("k", "A"),
                    vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(2.0))],
                    Some(vec![Stmt::If(
                        str_eq("k", "B"),
                        vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(3.0))],
                        Some(vec![Stmt::If(
                            str_eq("k", "C"),
                            vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(4.0))],
                            None,
                        )]),
                    )]),
                )]),
            ),
            Stmt::Return(Some(jsast::id("r"))),
        ],
    };
    run_speed_post_opts(&mut dup_key);
    assert!(
        !jsast::stmt_to_string(&dup_key).contains("switch"),
        "duplicate string key must bail out cleanly"
    );

    // 5. Negative C: Non-exiting `if` inside an `else` followed by a shared statement MUST NOT
    //    drop the shared statement for that key!
    let mut fallthrough_else = Stmt::FunctionDecl {
        name: "fallthrough".to_owned(),
        params: vec!["k".to_owned()],
        body: vec![
            jsast::lets(vec![("r".to_owned(), Some(Expr::Num(0.0)))]),
            Stmt::If(
                str_eq("k", "A"),
                vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(1.0))],
                Some(vec![Stmt::If(
                    str_eq("k", "B"),
                    vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(2.0))],
                    Some(vec![Stmt::If(
                        str_eq("k", "C"),
                        vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(3.0))],
                        Some(vec![
                            Stmt::If(
                                str_eq("k", "D"),
                                vec![jsast::assign_stmt(jsast::id("r"), Expr::Num(4.0))],
                                None,
                            ),
                            jsast::assign_stmt(
                                jsast::id("r"),
                                jsast::binary(BinOp::Add, jsast::id("r"), Expr::Num(10.0)),
                            ),
                        ]),
                    )]),
                )]),
            ),
            Stmt::Return(Some(jsast::id("r"))),
        ],
    };
    run_speed_post_opts(&mut fallthrough_else);
    assert!(
        !jsast::stmt_to_string(&fallthrough_else).contains("switch"),
        "non-exiting if inside else followed by shared tail must not miscompile (only 3 safe arms)"
    );
}

#[test]
fn coalesces_disjoint_switch_locals_while_preserving_overlapping_try_and_entry_live() {
    let params = vec!["k".to_owned(), "x".to_owned()];
    // 4 disjoint switch arms using t1..t4, plus `live_overlap` live across `t5`,
    // `handler_live` read in catch after `try_write`, and `entry_undef` read without write.
    let mut body = vec![
        jsast::lets(vec![
            ("t1".to_owned(), None),
            ("t2".to_owned(), None),
            ("t3".to_owned(), None),
            ("t4".to_owned(), None),
            ("live_overlap".to_owned(), None),
            ("t5".to_owned(), None),
            ("handler_live".to_owned(), None),
            ("try_write".to_owned(), None),
            ("entry_undef".to_owned(), None),
            ("res".to_owned(), None),
        ]),
        jsast::assign_stmt(jsast::id("handler_live"), jsast::id("x")),
        Stmt::TryCatch {
            body: vec![
                jsast::assign_stmt(
                    jsast::id("try_write"),
                    jsast::call(jsast::id("may_throw"), vec![jsast::id("x")]),
                ),
                jsast::assign_stmt(jsast::id("res"), jsast::id("try_write")),
            ],
            binding: "e".to_owned(),
            handler: vec![jsast::assign_stmt(
                jsast::id("res"),
                jsast::id("handler_live"),
            )],
        },
        Stmt::Switch(
            jsast::id("k"),
            vec![
                jsast::SwitchCase {
                    tests: vec![Expr::Num(1.0)],
                    body: vec![
                        jsast::assign_stmt(
                            jsast::id("t1"),
                            jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(1.0)),
                        ),
                        jsast::assign_stmt(jsast::id("res"), jsast::id("t1")),
                        Stmt::Break(None),
                    ],
                },
                jsast::SwitchCase {
                    tests: vec![Expr::Num(2.0)],
                    body: vec![
                        jsast::assign_stmt(
                            jsast::id("t2"),
                            jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(2.0)),
                        ),
                        jsast::assign_stmt(jsast::id("res"), jsast::id("t2")),
                        Stmt::Break(None),
                    ],
                },
                jsast::SwitchCase {
                    tests: vec![Expr::Num(3.0)],
                    body: vec![
                        jsast::assign_stmt(
                            jsast::id("t3"),
                            jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(3.0)),
                        ),
                        jsast::assign_stmt(jsast::id("res"), jsast::id("t3")),
                        Stmt::Break(None),
                    ],
                },
                jsast::SwitchCase {
                    tests: vec![],
                    body: vec![
                        jsast::assign_stmt(
                            jsast::id("t4"),
                            jsast::binary(BinOp::Add, jsast::id("res"), Expr::Num(4.0)),
                        ),
                        jsast::assign_stmt(jsast::id("live_overlap"), jsast::id("t4")),
                        jsast::assign_stmt(
                            jsast::id("t5"),
                            jsast::binary(BinOp::Add, jsast::id("x"), Expr::Num(5.0)),
                        ),
                        jsast::assign_stmt(
                            jsast::id("res"),
                            jsast::binary(BinOp::Add, jsast::id("live_overlap"), jsast::id("t5")),
                        ),
                    ],
                },
            ],
        ),
        Stmt::Return(Some(jsast::binary(
            BinOp::Add,
            jsast::id("res"),
            jsast::id("entry_undef"),
        ))),
    ];

    coalesce_disjoint_locals_with_threshold(&params, &mut body, 4);
    let decl_count = match &body[0] {
        Stmt::Lets(decls) => decls.len(),
        _ => panic!("expected leading Lets"),
    };
    let printed = jsast::stmt_to_string(&Stmt::FunctionDecl {
        name: "f".to_owned(),
        params,
        body,
    });
    assert!(
        decl_count <= 4,
        "expected 10 locals to coalesce into <= 4 slots (got {decl_count}): {printed}"
    );
    // `entry_undef` was read before write so it MUST stay its own untouched slot.
    assert!(
        printed.contains("entry_undef"),
        "entry-live local must not be coalesced: {printed}"
    );
    // `handler_live` and `try_write` interfere across the try/catch edge.
    let try_pos = printed.find("may_throw").unwrap();
    let catch_pos = printed.find("catch").unwrap();
    let try_slice = &printed[try_pos - 20..try_pos];
    let catch_slice = &printed[catch_pos..catch_pos + 50];
    assert_ne!(
        try_slice.split('=').next().unwrap().trim(),
        catch_slice
            .split('=')
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim(),
        "try_write must not alias handler_live: {printed}"
    );

    // `coalesce_disjoint_locals_decl` only activates for directly self-recursive functions.
    assert!(!body_calls_self(
        "non_rec",
        &[Stmt::Return(Some(jsast::call(jsast::id("other"), vec![])))]
    ));
    assert!(body_calls_self(
        "self_rec",
        &[Stmt::Return(Some(jsast::call(
            jsast::id("self_rec"),
            vec![]
        )))]
    ));
}

struct ToInt32DiamondSpec<'a> {
    label: &'a str,
    src: &'a str,
    dst: &'a str,
    abs_var: &'a str,
    rem_var: &'a str,
    mag_var: &'a str,
    neg_var: &'a str,
    res_var: &'a str,
    modulus: f64,
    unsigned_dst: bool,
}

/// Helper building the exact Rust `f64 -> i32/u32` `ToInt32` labeled-block diamond emitted
/// in `earcut` (`earcut_linked` and `is_ear_hashed_sl`).
fn make_to_int32_diamond(spec: ToInt32DiamondSpec<'_>) -> Stmt {
    let ToInt32DiamondSpec {
        label,
        src,
        dst,
        abs_var,
        rem_var,
        mag_var,
        neg_var,
        res_var,
        modulus,
        unsigned_dst,
    } = spec;
    let wrap_op = if unsigned_dst {
        BinOp::UShr
    } else {
        BinOp::BitOr
    };
    let wrap = |v: &str| Expr::Binary(wrap_op, Box::new(jsast::id(v)), Box::new(Expr::Num(0.0)));
    Stmt::Labeled(
        label.to_owned(),
        Box::new(Stmt::Block(vec![
            Stmt::If(
                Expr::Binary(
                    BinOp::And,
                    Box::new(Expr::Binary(
                        BinOp::Ge,
                        Box::new(jsast::id(src)),
                        Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
                    )),
                    Box::new(Expr::Binary(
                        BinOp::Le,
                        Box::new(jsast::id(src)),
                        Box::new(Expr::Num(2147483647.0)),
                    )),
                ),
                vec![
                    jsast::assign_stmt(jsast::id(dst), wrap(src)),
                    Stmt::Break(Some(label.to_owned())),
                ],
                None,
            ),
            jsast::assign_stmt(
                jsast::id(abs_var),
                jsast::method_call(jsast::id("Math"), "abs", vec![jsast::id(src)]),
            ),
            jsast::assign_stmt(
                jsast::id(mag_var),
                Expr::Seq(vec![
                    Expr::Assign(
                        Box::new(jsast::id(rem_var)),
                        Box::new(Expr::Binary(
                            BinOp::Rem,
                            Box::new(jsast::id(abs_var)),
                            Box::new(Expr::Num(modulus)),
                        )),
                    ),
                    Expr::Cond(
                        Box::new(Expr::Binary(
                            BinOp::Ge,
                            Box::new(jsast::id(rem_var)),
                            Box::new(Expr::Num(4294967295.0)),
                        )),
                        Box::new(Expr::Num(4294967295.0)),
                        Box::new(Expr::Cond(
                            Box::new(Expr::Binary(
                                BinOp::Gt,
                                Box::new(jsast::id(rem_var)),
                                Box::new(Expr::Num(0.0)),
                            )),
                            Box::new(Expr::Binary(
                                BinOp::UShr,
                                Box::new(jsast::id(rem_var)),
                                Box::new(Expr::Num(0.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        )),
                    ),
                ]),
            ),
            Stmt::If(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id(src)),
                    Box::new(Expr::Num(0.0)),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id(neg_var),
                        Expr::Binary(
                            BinOp::BitOr,
                            Box::new(Expr::Binary(
                                BinOp::Sub,
                                Box::new(Expr::Num(0.0)),
                                Box::new(jsast::id(mag_var)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    jsast::assign_stmt(jsast::id(res_var), wrap(neg_var)),
                ],
                Some(vec![jsast::assign_stmt(jsast::id(res_var), wrap(mag_var))]),
            ),
            jsast::assign_stmt(jsast::id(dst), jsast::id(res_var)),
        ])),
    )
}

#[test]
fn folds_to_int32_diamonds_signed_unsigned_and_back_to_back_reused_locals() {
    // 1. Disjoint locals (`is_ear_hashed_sl` shape) + unsigned `>>> 0` variant.
    let mut decl = Stmt::FunctionDecl {
        name: "zorder_disjoint".to_owned(),
        params: vec!["u".to_owned(), "w".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("S".to_owned(), None),
                ("U".to_owned(), None),
                ("aS".to_owned(), None),
                ("aL".to_owned(), None),
                ("E".to_owned(), None),
                ("aT".to_owned(), None),
                ("as_".to_owned(), None),
                ("bS".to_owned(), None),
                ("bL".to_owned(), None),
                ("F".to_owned(), None),
                ("bT".to_owned(), None),
                ("bs_".to_owned(), None),
            ]),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "j",
                src: "u",
                dst: "S",
                abs_var: "aS",
                rem_var: "E",
                mag_var: "aL",
                neg_var: "aT",
                res_var: "as_",
                modulus: 4294967296.0,
                unsigned_dst: false,
            }),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "a",
                src: "w",
                dst: "U",
                abs_var: "bS",
                rem_var: "F",
                mag_var: "bL",
                neg_var: "bT",
                res_var: "bs_",
                modulus: 4294967296.0,
                unsigned_dst: true,
            }),
            Stmt::Return(Some(Expr::Array(vec![jsast::id("S"), jsast::id("U")]))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        !printed.contains("4294967296") && !printed.contains("Math.abs"),
        "expected both signed & unsigned ToInt32 diamonds folded: {printed}"
    );
    assert!(
        printed.contains("S = u | 0;") && printed.contains("U = w >>> 0;"),
        "expected exact `S = u | 0` and `U = w >>> 0`: {printed}"
    );

    // 2. Coalesced back-to-back shape (`earcut_linked`): first diamond clobbers `m` & `j`,
    //    second diamond reads `g` and clobbers `m` & `g`, then both `g` & `m` are overwritten
    //    before any subsequent read.
    let mut coalesced = Stmt::FunctionDecl {
        name: "zorder_coalesced".to_owned(),
        params: vec!["x".to_owned(), "y".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("m".to_owned(), None),
                ("g".to_owned(), None),
                ("k".to_owned(), None),
                ("h".to_owned(), None),
                ("j".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("m"), jsast::id("x")),
            jsast::assign_stmt(jsast::id("g"), jsast::id("y")),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "d",
                src: "m",
                dst: "k",
                abs_var: "j",
                rem_var: "j",
                mag_var: "k",
                neg_var: "m",
                res_var: "m",
                modulus: 4294967296.0,
                unsigned_dst: false,
            }),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "l",
                src: "g",
                dst: "h",
                abs_var: "m",
                rem_var: "m",
                mag_var: "h",
                neg_var: "g",
                res_var: "g",
                modulus: 4294967296.0,
                unsigned_dst: false,
            }),
            jsast::assign_stmt(
                jsast::id("g"),
                Expr::Binary(
                    BinOp::BitAnd,
                    Box::new(jsast::id("k")),
                    Box::new(Expr::Num(255.0)),
                ),
            ),
            jsast::assign_stmt(
                jsast::id("m"),
                Expr::Binary(
                    BinOp::BitAnd,
                    Box::new(jsast::id("h")),
                    Box::new(Expr::Num(255.0)),
                ),
            ),
            Stmt::Return(Some(Expr::Binary(
                BinOp::BitOr,
                Box::new(jsast::id("g")),
                Box::new(jsast::id("m")),
            ))),
        ],
    };
    run_speed_post_opts(&mut coalesced);
    let coalesced_js = jsast::stmt_to_string(&coalesced);
    assert!(
        !coalesced_js.contains("4294967296") && !coalesced_js.contains("Math.abs"),
        "expected back-to-back coalesced ToInt32 diamonds folded: {coalesced_js}"
    );
    assert!(
        coalesced_js.contains("k = m | 0;") && coalesced_js.contains("h = g | 0;"),
        "expected `k = m | 0` and `h = g | 0`: {coalesced_js}"
    );
}

#[test]
fn rejects_unsound_to_int32_diamonds_live_clobber_shadowed_math_wrong_mod() {
    // 1. Live temp `j` read after the block MUST NOT fold.
    let mut live_temp = Stmt::FunctionDecl {
        name: "live_temp".to_owned(),
        params: vec!["m".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("k".to_owned(), None),
                ("j".to_owned(), None),
                ("n".to_owned(), None),
                ("r".to_owned(), None),
            ]),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "d",
                src: "m",
                dst: "k",
                abs_var: "j",
                rem_var: "j",
                mag_var: "k",
                neg_var: "n",
                res_var: "r",
                modulus: 4294967296.0,
                unsigned_dst: false,
            }),
            Stmt::Return(Some(Expr::Array(vec![jsast::id("k"), jsast::id("j")]))),
        ],
    };
    run_speed_post_opts(&mut live_temp);
    let s1 = jsast::stmt_to_string(&live_temp);
    assert!(
        s1.contains("4294967296"),
        "live temp `j` must prevent folding: {s1}"
    );

    // 2. Live clobbered `src` (`m`) read after the block MUST NOT fold.
    let mut live_src = Stmt::FunctionDecl {
        name: "live_src".to_owned(),
        params: vec!["m".to_owned()],
        body: vec![
            jsast::lets(vec![("k".to_owned(), None), ("j".to_owned(), None)]),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "d",
                src: "m",
                dst: "k",
                abs_var: "j",
                rem_var: "j",
                mag_var: "k",
                neg_var: "m",
                res_var: "m",
                modulus: 4294967296.0,
                unsigned_dst: false,
            }),
            Stmt::Return(Some(Expr::Array(vec![jsast::id("k"), jsast::id("m")]))),
        ],
    };
    run_speed_post_opts(&mut live_src);
    let s2 = jsast::stmt_to_string(&live_src);
    assert!(
        s2.contains("4294967296"),
        "live clobbered `m` must prevent folding: {s2}"
    );

    // 3. Shadowed `Math` parameter MUST NOT fold.
    let mut shadowed_math = Stmt::FunctionDecl {
        name: "shadowed_math".to_owned(),
        params: vec!["Math".to_owned(), "u".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("S".to_owned(), None),
                ("aS".to_owned(), None),
                ("aL".to_owned(), None),
                ("E".to_owned(), None),
                ("aT".to_owned(), None),
                ("as_".to_owned(), None),
            ]),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "j",
                src: "u",
                dst: "S",
                abs_var: "aS",
                rem_var: "E",
                mag_var: "aL",
                neg_var: "aT",
                res_var: "as_",
                modulus: 4294967296.0,
                unsigned_dst: false,
            }),
            Stmt::Return(Some(jsast::id("S"))),
        ],
    };
    run_speed_post_opts(&mut shadowed_math);
    let s3 = jsast::stmt_to_string(&shadowed_math);
    assert!(
        s3.contains("4294967296"),
        "shadowed Math must prevent folding: {s3}"
    );

    // 4. Wrong modulus (`4294967295` instead of `4294967296`) MUST NOT fold.
    let mut wrong_mod = Stmt::FunctionDecl {
        name: "wrong_mod".to_owned(),
        params: vec!["u".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("S".to_owned(), None),
                ("aS".to_owned(), None),
                ("aL".to_owned(), None),
                ("E".to_owned(), None),
                ("aT".to_owned(), None),
                ("as_".to_owned(), None),
            ]),
            make_to_int32_diamond(ToInt32DiamondSpec {
                label: "j",
                src: "u",
                dst: "S",
                abs_var: "aS",
                rem_var: "E",
                mag_var: "aL",
                neg_var: "aT",
                res_var: "as_",
                modulus: 4294967295.0,
                unsigned_dst: false,
            }),
            Stmt::Return(Some(jsast::id("S"))),
        ],
    };
    run_speed_post_opts(&mut wrong_mod);
    let s4 = jsast::stmt_to_string(&wrong_mod);
    assert!(
        s4.contains("4294967295"),
        "wrong modulus must prevent folding: {s4}"
    );
}

#[test]
fn algebraic_post_opts_i32_cond_array_index_neg_bool_and_self_assign() {
    let mut decl = Stmt::FunctionDecl {
        name: "cleanups".to_owned(),
        params: vec![
            "x".to_owned(),
            "y".to_owned(),
            "n".to_owned(),
            "f".to_owned(),
        ],
        body: vec![
            jsast::lets(vec![
                ("c1".to_owned(), None),
                ("c2".to_owned(), None),
                ("c3".to_owned(), None),
                ("c4".to_owned(), None),
                ("c5".to_owned(), None),
                ("c6".to_owned(), None),
                ("c7".to_owned(), None),
                ("c8".to_owned(), None),
                ("F".to_owned(), None),
            ]),
            // 1. `(x <= -2147483648 ? -2147483648 : x >= 2147483647 ? 2147483647 : x | 0) | 0`
            //    drops outer redundant `| 0`.
            jsast::assign_stmt(
                jsast::id("c1"),
                Expr::Binary(
                    BinOp::BitOr,
                    Box::new(Expr::Cond(
                        Box::new(Expr::Binary(
                            BinOp::Le,
                            Box::new(jsast::id("x")),
                            Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
                        )),
                        Box::new(Expr::Unary(UnOp::Neg, Box::new(Expr::Num(2147483648.0)))),
                        Box::new(Expr::Cond(
                            Box::new(Expr::Binary(
                                BinOp::Ge,
                                Box::new(jsast::id("x")),
                                Box::new(Expr::Num(2147483647.0)),
                            )),
                            Box::new(Expr::Num(2147483647.0)),
                            Box::new(Expr::Binary(
                                BinOp::BitOr,
                                Box::new(jsast::id("x")),
                                Box::new(Expr::Num(0.0)),
                            )),
                        )),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            // 2. `(x >>> 12) | 0` -> `x >>> 12` (proven nonneg i31)
            jsast::assign_stmt(
                jsast::id("c2"),
                Expr::Binary(
                    BinOp::BitOr,
                    Box::new(Expr::Binary(
                        BinOp::UShr,
                        Box::new(jsast::id("x")),
                        Box::new(Expr::Num(12.0)),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            // 3. `(0 - y) | 0` -> `-y | 0`
            jsast::assign_stmt(
                jsast::id("c3"),
                Expr::Binary(
                    BinOp::BitOr,
                    Box::new(Expr::Binary(
                        BinOp::Sub,
                        Box::new(Expr::Num(0.0)),
                        Box::new(jsast::id("y")),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            // 4. `[void 0, y][1]` -> `y`, while `[f(), y][1]` MUST NOT fold (impure discarded elem)
            //    and `[c4, y][0] = 7` MUST NOT turn LHS into non-lvalue/direct `c4 = 7`.
            jsast::assign_stmt(
                jsast::id("c4"),
                jsast::index(
                    Expr::Array(vec![Expr::Undefined, jsast::id("y")]),
                    Expr::Num(1.0),
                ),
            ),
            jsast::assign_stmt(
                jsast::id("c5"),
                jsast::index(
                    Expr::Array(vec![jsast::call(jsast::id("f"), vec![]), jsast::id("y")]),
                    Expr::Num(1.0),
                ),
            ),
            jsast::assign_stmt(
                jsast::index(
                    Expr::Array(vec![jsast::id("c4"), jsast::id("y")]),
                    Expr::Num(0.0),
                ),
                Expr::Num(7.0),
            ),
            // 5. `(x === 1) && true` -> `x === 1`; non-bool `n && true` MUST NOT fold.
            jsast::assign_stmt(
                jsast::id("c6"),
                Expr::Binary(
                    BinOp::And,
                    Box::new(Expr::Binary(
                        BinOp::StrictEq,
                        Box::new(jsast::id("x")),
                        Box::new(Expr::Num(1.0)),
                    )),
                    Box::new(Expr::Bool(true)),
                ),
            ),
            jsast::assign_stmt(
                jsast::id("c7"),
                Expr::Binary(
                    BinOp::And,
                    Box::new(jsast::id("n")),
                    Box::new(Expr::Bool(true)),
                ),
            ),
            // 6. `if (!!x) { c8 = 1; }` -> `if (x) { c8 = 1; }`
            Stmt::If(
                Expr::Unary(
                    UnOp::Not,
                    Box::new(Expr::Unary(UnOp::Not, Box::new(jsast::id("x")))),
                ),
                vec![jsast::assign_stmt(jsast::id("c8"), Expr::Num(1.0))],
                None,
            ),
            // 7. `if (F < y) { F = F; } else { F = y; }` -> `if (!(F < y)) { F = y; }`
            Stmt::If(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("F")),
                    Box::new(jsast::id("y")),
                ),
                vec![jsast::assign_stmt(jsast::id("F"), jsast::id("F"))],
                Some(vec![jsast::assign_stmt(jsast::id("F"), jsast::id("y"))]),
            ),
            Stmt::Return(Some(Expr::Array(vec![
                jsast::id("c1"),
                jsast::id("c2"),
                jsast::id("c3"),
                jsast::id("c4"),
                jsast::id("c5"),
                jsast::id("c6"),
                jsast::id("c7"),
                jsast::id("c8"),
                jsast::id("F"),
            ]))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let printed = jsast::stmt_to_string(&decl);
    assert!(
        printed.contains(
            "c1 = x <= -2147483648 ? -2147483648 : x >= 2147483647 ? 2147483647 : x | 0;"
        ),
        "expected outer `| 0` dropped on i32 ternary: {printed}"
    );
    assert!(
        printed.contains("c2 = x >>> 12;"),
        "expected `(x >>> 12) | 0` folded to `x >>> 12`: {printed}"
    );
    assert!(
        printed.contains("c3 = -y | 0;"),
        "expected `(0 - y) | 0` folded to `-y | 0`: {printed}"
    );
    assert!(
        printed.contains("c4 = y;"),
        "expected `[void 0, y][1]` folded to `y`: {printed}"
    );
    assert!(
        printed.contains("c5 = [f(), y][1];"),
        "impure discarded element must prevent array index folding: {printed}"
    );
    assert!(
        printed.contains("[c4, y][0] = 7;"),
        "assignment LHS array index must NOT fold: {printed}"
    );
    assert!(
        printed.contains("c6 = x === 1;"),
        "expected `bool && true` folded: {printed}"
    );
    assert!(
        printed.contains("c7 = n && true;"),
        "non-bool `n && true` must NOT fold: {printed}"
    );
    assert!(
        printed.contains("if (x)") && !printed.contains("!!x"),
        "expected `if (!!x)` unwrapped to `if (x)`: {printed}"
    );
    assert!(
        printed.contains("if (!(F < y))") && !printed.contains("F = F"),
        "expected self-assign removed and empty-then `if` inverted: {printed}"
    );
}
