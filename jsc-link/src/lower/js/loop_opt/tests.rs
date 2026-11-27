#![allow(
    clippy::too_many_lines,
    reason = "unit tests build multi-statement loop AST fixtures inline"
)]

use super::super::post_opt::run_speed_post_opts;
use rcgjs_object::jsast::{self, BinOp, Expr, Stmt, UnOp};

#[test]
fn folds_guarded_f2i_clamp_to_bitor_zero() {
    let mut decl = Stmt::FunctionDecl {
        name: "f".to_owned(),
        params: vec!["n".to_owned()],
        body: vec![
            jsast::lets(vec![("S".to_owned(), None)]),
            Stmt::If(
                Expr::Binary(
                    BinOp::Ge,
                    Box::new(jsast::id("n")),
                    Box::new(Expr::Num(-2147483648.0)),
                ),
                vec![Stmt::If(
                    Expr::Binary(
                        BinOp::Le,
                        Box::new(jsast::id("n")),
                        Box::new(Expr::Num(2147483647.0)),
                    ),
                    vec![jsast::assign_stmt(
                        jsast::id("S"),
                        jsast::cond(
                            Expr::Binary(
                                BinOp::Le,
                                Box::new(jsast::id("n")),
                                Box::new(Expr::Num(-2147483648.0)),
                            ),
                            Expr::Num(-2147483648.0),
                            jsast::cond(
                                Expr::Binary(
                                    BinOp::Ge,
                                    Box::new(jsast::id("n")),
                                    Box::new(Expr::Num(2147483647.0)),
                                ),
                                Expr::Num(2147483647.0),
                                Expr::Binary(
                                    BinOp::BitOr,
                                    Box::new(jsast::id("n")),
                                    Box::new(Expr::Num(0.0)),
                                ),
                            ),
                        ),
                    )],
                    None,
                )],
                None,
            ),
            Stmt::Return(Some(jsast::id("S"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(s.contains("S = n | 0;"), "expected S = n | 0: {s}");
    assert!(!s.contains("?"), "expected clamp ternary gone: {s}");
}

#[test]
fn versions_fflate_match_loop_and_hoists_slice_props() {
    let mut decl = Stmt::FunctionDecl {
        name: "match_loop".to_owned(),
        params: vec![
            "a".to_owned(),
            "g".to_owned(),
            "l".to_owned(),
            "bh".to_owned(),
        ],
        body: vec![
            jsast::lets(vec![
                ("h".to_owned(), None),
                ("j".to_owned(), None),
                ("ak".to_owned(), None),
                ("al".to_owned(), None),
                ("br".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("h"), jsast::member(jsast::id("a"), "len")),
            jsast::assign_stmt(jsast::id("j"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("j")),
                    Box::new(jsast::id("bh")),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("ak"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("g")),
                                Box::new(jsast::id("j")),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    Stmt::If(
                        Expr::Unary(
                            UnOp::Not,
                            Box::new(Expr::Binary(
                                BinOp::Lt,
                                Box::new(jsast::id("ak")),
                                Box::new(jsast::id("h")),
                            )),
                        ),
                        vec![Stmt::ExprStmt(jsast::call(
                            jsast::id("core$panicking$panic_bounds_check$h1"),
                            vec![jsast::id("ak"), jsast::id("h")],
                        ))],
                        None,
                    ),
                    jsast::assign_stmt(
                        jsast::id("br"),
                        jsast::index(
                            jsast::member(jsast::id("a"), "buf"),
                            Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::member(jsast::id("a"), "off")),
                                Box::new(jsast::id("ak")),
                            ),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("al"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Sub,
                                Box::new(Expr::Binary(
                                    BinOp::BitOr,
                                    Box::new(Expr::Binary(
                                        BinOp::Add,
                                        Box::new(jsast::id("g")),
                                        Box::new(jsast::id("j")),
                                    )),
                                    Box::new(Expr::Num(0.0)),
                                )),
                                Box::new(jsast::id("l")),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    Stmt::If(
                        Expr::Unary(
                            UnOp::Not,
                            Box::new(Expr::Binary(
                                BinOp::Lt,
                                Box::new(jsast::id("al")),
                                Box::new(jsast::id("h")),
                            )),
                        ),
                        vec![Stmt::ExprStmt(jsast::call(
                            jsast::id("core$panicking$panic_bounds_check$h1"),
                            vec![jsast::id("al"), jsast::id("h")],
                        ))],
                        None,
                    ),
                    jsast::assign_stmt(
                        jsast::id("j"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("j")),
                                Box::new(Expr::Num(1.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                ],
            ),
            Stmt::Return(Some(jsast::id("j"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(s.contains("$hb_0 = a.buf;"), "expected a.buf hoisted: {s}");
    assert!(s.contains("$ho_0 = a.off;"), "expected a.off hoisted: {s}");
    assert_eq!(
        s.matches("while (j < bh)").count(),
        2,
        "expected fast+slow loop versioning: {s}"
    );
    assert_eq!(
        s.matches("panic_bounds_check").count(),
        2,
        "expected panic checks only in fallback branch: {s}"
    );
}

#[test]
fn unrolls_small_const_loop_and_folds_indices() {
    let mut decl = Stmt::FunctionDecl {
        name: "f".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("m".to_owned(), None),
                ("next".to_owned(), None),
                ("s".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("m"), Expr::Num(0.0)),
            Stmt::LoopForever(
                None,
                vec![Stmt::If(
                    Expr::Binary(
                        BinOp::Lt,
                        Box::new(jsast::id("m")),
                        Box::new(Expr::Num(5.0)),
                    ),
                    vec![
                        jsast::assign_stmt(
                            jsast::id("next"),
                            Expr::Binary(
                                BinOp::UShr,
                                Box::new(Expr::Binary(
                                    BinOp::Add,
                                    Box::new(jsast::id("m")),
                                    Box::new(Expr::Num(1.0)),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                        ),
                        jsast::assign_stmt(
                            jsast::id("s"),
                            jsast::method_call(
                                jsast::id("Math"),
                                "imul",
                                vec![Expr::Num(2.0), jsast::id("m")],
                            ),
                        ),
                        jsast::assign_stmt(
                            jsast::index(jsast::id("a"), jsast::id("s")),
                            jsast::id("m"),
                        ),
                        jsast::assign_stmt(jsast::id("m"), jsast::id("next")),
                    ],
                    Some(vec![Stmt::Break(None)]),
                )],
            ),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(!s.contains("for (;;)"), "expected loop unrolled: {s}");
    assert!(!s.contains("Math.imul"), "expected Math.imul folded: {s}");
    assert!(s.contains("a[8] = 4;"), "expected a[8] = 4: {s}");
}

#[test]
fn elides_shallow_obj_map_clones_and_versions_through_pure_arrow() {
    let mut decl = Stmt::FunctionDecl {
        name: "compress_like".to_owned(),
        params: vec![
            "g".to_owned(),
            "i".to_owned(),
            "p".to_owned(),
            "q".to_owned(),
        ],
        body: vec![
            jsast::lets(vec![
                ("l".to_owned(), None),
                ("b9".to_owned(), None),
                ("bp".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("l"), Expr::Num(16.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("l")),
                    Box::new(Expr::Num(64.0)),
                ),
                vec![
                    Stmt::If(
                        Expr::Unary(
                            UnOp::Not,
                            Box::new(Expr::Binary(
                                BinOp::Lt,
                                Box::new(Expr::Binary(
                                    BinOp::Sub,
                                    Box::new(jsast::id("l")),
                                    Box::new(Expr::Num(15.0)),
                                )),
                                Box::new(jsast::id("i")),
                            )),
                        ),
                        vec![Stmt::ExprStmt(jsast::call(
                            jsast::id("core$panicking$panic_bounds_check$h1"),
                            vec![jsast::id("l"), jsast::id("i")],
                        ))],
                        None,
                    ),
                    Stmt::If(
                        Expr::Unary(
                            UnOp::Not,
                            Box::new(Expr::Binary(
                                BinOp::Lt,
                                Box::new(jsast::id("l")),
                                Box::new(jsast::id("i")),
                            )),
                        ),
                        vec![Stmt::ExprStmt(jsast::call(
                            jsast::id("core$panicking$panic_bounds_check$h1"),
                            vec![jsast::id("l"), jsast::id("i")],
                        ))],
                        None,
                    ),
                    jsast::assign_stmt(
                        jsast::id("l"),
                        Expr::Binary(
                            BinOp::Add,
                            Box::new(jsast::id("l")),
                            Box::new(Expr::Num(1.0)),
                        ),
                    ),
                ],
            ),
            jsast::assign_stmt(
                jsast::id("b9"),
                Expr::Array(vec![Expr::Object(vec![
                    ("hi".to_owned(), jsast::id("p")),
                    ("lo".to_owned(), jsast::id("q")),
                ])]),
            ),
            jsast::assign_stmt(
                jsast::id("bp"),
                jsast::method_call(
                    jsast::id("b9"),
                    "map",
                    vec![Expr::Arrow(
                        vec!["m".to_owned()],
                        jsast::ArrowBody::Expr(Box::new(Expr::Object(vec![
                            ("hi".to_owned(), jsast::member(jsast::id("m"), "hi")),
                            ("lo".to_owned(), jsast::member(jsast::id("m"), "lo")),
                        ]))),
                    )],
                ),
            ),
            Stmt::Return(Some(jsast::method_call(
                jsast::id("bp"),
                "map",
                vec![Expr::Arrow(
                    vec!["m".to_owned()],
                    jsast::ArrowBody::Expr(Box::new(jsast::cond(
                        Expr::Binary(
                            BinOp::StrictNe,
                            Box::new(jsast::id("m")),
                            Box::new(Expr::Undefined),
                        ),
                        Expr::Object(vec![
                            ("hi".to_owned(), jsast::member(jsast::id("m"), "hi")),
                            ("lo".to_owned(), jsast::member(jsast::id("m"), "lo")),
                        ]),
                        Expr::Undefined,
                    ))),
                )],
            ))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(
        !s.contains(".map("),
        "expected both .map() clones elided: {s}"
    );
    assert!(
        s.contains("if (63 < i)"),
        "expected simplified single guard 63 < i: {s}"
    );
    assert!(
        !s.contains("48 < i"),
        "expected dominated 48 < i dropped: {s}"
    );
}

#[test]
fn hoists_as_mut_slice_and_versions_labeled_masked_probe_loop() {
    let mut decl = Stmt::FunctionDecl {
        name: "refine_like".to_owned(),
        params: vec!["aw".to_owned(), "av".to_owned(), "h0".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("r".to_owned(), None),
                ("q".to_owned(), None),
                ("G".to_owned(), None),
            ]),
            jsast::assign_stmt(
                jsast::id("r"),
                jsast::call(
                    jsast::id("alloc$vec$impl_1_as_mut_slice$h1234"),
                    vec![jsast::id("aw")],
                ),
            ),
            jsast::assign_stmt(
                jsast::id("q"),
                Expr::Binary(
                    BinOp::UShr,
                    Box::new(Expr::Binary(
                        BinOp::BitAnd,
                        Box::new(jsast::id("h0")),
                        Box::new(jsast::id("av")),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            Stmt::Labeled(
                "a".to_owned(),
                Box::new(Stmt::Block(vec![Stmt::LoopForever(
                    None,
                    vec![
                        jsast::assign_stmt(jsast::id("G"), jsast::member(jsast::id("r"), "len")),
                        Stmt::If(
                            Expr::Unary(
                                UnOp::Not,
                                Box::new(Expr::Binary(
                                    BinOp::Lt,
                                    Box::new(jsast::id("q")),
                                    Box::new(jsast::id("G")),
                                )),
                            ),
                            vec![Stmt::ExprStmt(jsast::call(
                                jsast::id("core$panicking$panic_bounds_check$h1"),
                                vec![jsast::id("q"), jsast::id("G")],
                            ))],
                            None,
                        ),
                        Stmt::If(
                            Expr::Binary(
                                BinOp::StrictEq,
                                Box::new(jsast::index(
                                    jsast::member(jsast::id("r"), "buf"),
                                    Expr::Binary(
                                        BinOp::Add,
                                        Box::new(jsast::member(jsast::id("r"), "off")),
                                        Box::new(jsast::id("q")),
                                    ),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                            vec![Stmt::Break(Some("a".to_owned()))],
                            None,
                        ),
                        jsast::assign_stmt(
                            jsast::id("q"),
                            Expr::Binary(
                                BinOp::UShr,
                                Box::new(Expr::Binary(
                                    BinOp::BitAnd,
                                    Box::new(Expr::Binary(
                                        BinOp::Add,
                                        Box::new(jsast::id("q")),
                                        Box::new(Expr::Num(1.0)),
                                    )),
                                    Box::new(jsast::id("av")),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                        ),
                    ],
                )])),
            ),
            Stmt::Return(Some(jsast::id("q"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(s.contains("$hb_0 = r.buf;"), "expected r.buf hoisted: {s}");
    assert!(s.contains("$hl_0 = r.len;"), "expected r.len hoisted: {s}");
    assert!(
        s.contains("av >>> 0 < $hl_0"),
        "expected pre-loop masked guard: {s}"
    );
    assert_eq!(
        s.matches("panic_bounds_check").count(),
        1,
        "expected bounds check stripped from fast loop: {s}"
    );
}

#[test]
fn straight_line_prop_does_not_leak_past_labeled_block_break() {
    // Reproduces 04_iter_range / 05_slice_index: a small unrolled loop triggers
    // `prop_straight_line_lits`, which must NOT propagate `res = "many"` past `a: { if (q) { res = "bookends"; break a; } res = "many"; }`.
    let mut decl = Stmt::FunctionDecl {
        name: "f".to_owned(),
        params: vec!["q".to_owned(), "a".to_owned()],
        body: vec![
            jsast::lets(vec![("m".to_owned(), None), ("res".to_owned(), None)]),
            jsast::assign_stmt(jsast::id("m"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("m")),
                    Box::new(Expr::Num(3.0)),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::index(jsast::id("a"), jsast::id("m")),
                        jsast::id("m"),
                    ),
                    jsast::assign_stmt(
                        jsast::id("m"),
                        Expr::Binary(
                            BinOp::Add,
                            Box::new(jsast::id("m")),
                            Box::new(Expr::Num(1.0)),
                        ),
                    ),
                ],
            ),
            Stmt::Labeled(
                "lbl".to_owned(),
                Box::new(Stmt::Block(vec![
                    Stmt::If(
                        jsast::id("q"),
                        vec![
                            jsast::assign_stmt(jsast::id("res"), Expr::Str("bookends".to_owned())),
                            Stmt::Break(Some("lbl".to_owned())),
                        ],
                        None,
                    ),
                    jsast::assign_stmt(jsast::id("res"), Expr::Str("many".to_owned())),
                ])),
            ),
            Stmt::Return(Some(jsast::id("res"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(
        s.contains("return res;"),
        "expected `res` preserved after labeled block break: {s}"
    );
    assert!(
        s.contains("\"bookends\""),
        "expected `bookends` branch preserved: {s}"
    );
}

#[test]
fn two_level_licm_and_add_const_bce_version_labeled_switch_dp_loop() {
    // Models `minisearch::recurse`: `a: switch (tag)` -> `b: for (;;)` -> inner DP `for (;;)`
    // whose `else` exits via `if (cond) { break; } else { continue b; }`, reading `a.matrix`
    // via `G = a.matrix; an = G.buf, ao = G.len;` with `l = ai + d >>> 0` and `y = l + 1 >>> 0`.
    let mut decl = Stmt::FunctionDecl {
        name: "dp".to_owned(),
        params: vec![
            "a".to_owned(),
            "tag".to_owned(),
            "ai".to_owned(),
            "bound".to_owned(),
        ],
        body: vec![
            jsast::lets(vec![
                ("o".to_owned(), None),
                ("F".to_owned(), None),
                ("aj".to_owned(), None),
                ("ak".to_owned(), None),
                ("d".to_owned(), None),
                ("am".to_owned(), None),
                ("L".to_owned(), None),
                ("l".to_owned(), None),
                ("y".to_owned(), None),
                ("G".to_owned(), None),
                ("an".to_owned(), None),
                ("ao".to_owned(), None),
                ("N".to_owned(), None),
                ("H".to_owned(), None),
                ("ap".to_owned(), None),
                ("aq".to_owned(), None),
                ("P".to_owned(), None),
                ("sum".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("o"), jsast::member(jsast::id("a"), "columns")),
            jsast::assign_stmt(jsast::id("sum"), Expr::Num(0.0)),
            Stmt::Labeled(
                "sw".to_owned(),
                Box::new(Stmt::Switch(
                    jsast::id("tag"),
                    vec![jsast::SwitchCase {
                        tests: vec![Expr::Num(1.0)],
                        body: vec![Stmt::Labeled(
                            "outer".to_owned(),
                            Box::new(Stmt::LoopForever(
                                None,
                                vec![
                                    jsast::assign_stmt(
                                        jsast::id("F"),
                                        jsast::member(jsast::id("a"), "matrix"),
                                    ),
                                    Stmt::ExprStmt(Expr::Seq(vec![
                                        jsast::assign(
                                            jsast::id("aj"),
                                            jsast::member(jsast::id("F"), "buf"),
                                        ),
                                        jsast::assign(
                                            jsast::id("ak"),
                                            jsast::member(jsast::id("F"), "len"),
                                        ),
                                    ])),
                                    jsast::assign_stmt(jsast::id("d"), Expr::Num(0.0)),
                                    Stmt::LoopForever(
                                        None,
                                        vec![
                                            jsast::assign_stmt(
                                                jsast::id("am"),
                                                Expr::Binary(
                                                    BinOp::Lt,
                                                    Box::new(jsast::id("d")),
                                                    Box::new(jsast::id("bound")),
                                                ),
                                            ),
                                            Stmt::If(
                                                jsast::id("am"),
                                                vec![
                                                    jsast::assign_stmt(
                                                        jsast::id("L"),
                                                        Expr::Binary(
                                                            BinOp::UShr,
                                                            Box::new(Expr::Binary(
                                                                BinOp::Add,
                                                                Box::new(jsast::id("d")),
                                                                Box::new(Expr::Num(1.0)),
                                                            )),
                                                            Box::new(Expr::Num(0.0)),
                                                        ),
                                                    ),
                                                    jsast::assign_stmt(
                                                        jsast::id("l"),
                                                        Expr::Binary(
                                                            BinOp::UShr,
                                                            Box::new(Expr::Binary(
                                                                BinOp::Add,
                                                                Box::new(jsast::id("ai")),
                                                                Box::new(jsast::id("d")),
                                                            )),
                                                            Box::new(Expr::Num(0.0)),
                                                        ),
                                                    ),
                                                    jsast::assign_stmt(
                                                        jsast::id("G"),
                                                        jsast::member(jsast::id("a"), "matrix"),
                                                    ),
                                                    Stmt::ExprStmt(Expr::Seq(vec![
                                                        jsast::assign(
                                                            jsast::id("an"),
                                                            jsast::member(jsast::id("G"), "buf"),
                                                        ),
                                                        jsast::assign(
                                                            jsast::id("ao"),
                                                            jsast::member(jsast::id("G"), "len"),
                                                        ),
                                                    ])),
                                                    jsast::assign_stmt(
                                                        jsast::id("N"),
                                                        jsast::id("ao"),
                                                    ),
                                                    Stmt::If(
                                                        Expr::Unary(
                                                            UnOp::Not,
                                                            Box::new(Expr::Binary(
                                                                BinOp::Lt,
                                                                Box::new(jsast::id("l")),
                                                                Box::new(jsast::id("N")),
                                                            )),
                                                        ),
                                                        vec![Stmt::Throw(jsast::call(
                                                            jsast::id("core$panicking$panic_bounds_check$h1"),
                                                            vec![jsast::id("l"), jsast::id("N")],
                                                        ))],
                                                        None,
                                                    ),
                                                    jsast::assign_stmt(
                                                        jsast::id("y"),
                                                        Expr::Binary(
                                                            BinOp::UShr,
                                                            Box::new(Expr::Binary(
                                                                BinOp::Add,
                                                                Box::new(jsast::id("l")),
                                                                Box::new(Expr::Num(1.0)),
                                                            )),
                                                            Box::new(Expr::Num(0.0)),
                                                        ),
                                                    ),
                                                    jsast::assign_stmt(
                                                        jsast::id("H"),
                                                        jsast::member(jsast::id("a"), "matrix"),
                                                    ),
                                                    Stmt::ExprStmt(Expr::Seq(vec![
                                                        jsast::assign(
                                                            jsast::id("ap"),
                                                            jsast::member(jsast::id("H"), "buf"),
                                                        ),
                                                        jsast::assign(
                                                            jsast::id("aq"),
                                                            jsast::member(jsast::id("H"), "len"),
                                                        ),
                                                    ])),
                                                    jsast::assign_stmt(
                                                        jsast::id("P"),
                                                        jsast::id("aq"),
                                                    ),
                                                    Stmt::If(
                                                        Expr::Unary(
                                                            UnOp::Not,
                                                            Box::new(Expr::Binary(
                                                                BinOp::Lt,
                                                                Box::new(jsast::id("y")),
                                                                Box::new(jsast::id("P")),
                                                            )),
                                                        ),
                                                        vec![Stmt::Throw(jsast::call(
                                                            jsast::id("core$panicking$panic_bounds_check$h1"),
                                                            vec![jsast::id("y"), jsast::id("P")],
                                                        ))],
                                                        None,
                                                    ),
                                                    jsast::assign_stmt(
                                                        jsast::id("sum"),
                                                        Expr::Binary(
                                                            BinOp::Add,
                                                            Box::new(jsast::index(
                                                                jsast::id("an"),
                                                                jsast::id("l"),
                                                            )),
                                                            Box::new(jsast::index(
                                                                jsast::id("ap"),
                                                                jsast::id("y"),
                                                            )),
                                                        ),
                                                    ),
                                                    jsast::assign_stmt(
                                                        jsast::id("d"),
                                                        jsast::id("L"),
                                                    ),
                                                ],
                                                Some(vec![Stmt::If(
                                                    Expr::Binary(
                                                        BinOp::Gt,
                                                        Box::new(jsast::id("sum")),
                                                        Box::new(jsast::id("o")),
                                                    ),
                                                    vec![Stmt::Break(None)],
                                                    Some(vec![Stmt::Continue(Some(
                                                        "outer".to_owned(),
                                                    ))]),
                                                )]),
                                            ),
                                        ],
                                    ),
                                    Stmt::Break(None),
                                ],
                            )),
                        )],
                    }],
                )),
            ),
            Stmt::Return(Some(jsast::id("sum"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(
        s.contains("ai + bound < $hl_") && !s.contains("ai + bound <= 4294967296"),
        "expected AddConst pre-loop guard with strictest RHS bound: {s}"
    );
    assert!(
        s.contains("ai + bound <= 4294967295"),
        "expected exact u32 upper bound for AddConst: {s}"
    );
    assert_eq!(
        s.matches("panic_bounds_check").count(),
        2,
        "expected both bounds checks stripped from fast loop (only 2 in cold fallback): {s}"
    );
}

#[test]
fn immutable_slice_hoists_across_call_and_replaces_lhs_member_base() {
    let mut decl = Stmt::FunctionDecl {
        name: "update".to_owned(),
        params: vec!["s".to_owned(), "n".to_owned()],
        body: vec![
            jsast::lets(vec![("i".to_owned(), None)]),
            jsast::assign_stmt(jsast::id("i"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("i")),
                    Box::new(jsast::id("n")),
                ),
                vec![
                    Stmt::ExprStmt(jsast::call(jsast::id("helper"), vec![jsast::id("s")])),
                    jsast::assign_stmt(
                        jsast::member(
                            jsast::index(
                                jsast::member(jsast::id("s"), "buf"),
                                Expr::Binary(
                                    BinOp::Add,
                                    Box::new(jsast::member(jsast::id("s"), "off")),
                                    Box::new(jsast::id("i")),
                                ),
                            ),
                            "x",
                        ),
                        jsast::member(
                            jsast::index(
                                jsast::member(jsast::id("s"), "buf"),
                                Expr::Binary(
                                    BinOp::Add,
                                    Box::new(jsast::member(jsast::id("s"), "off")),
                                    Box::new(jsast::id("i")),
                                ),
                            ),
                            "y",
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("i"),
                        Expr::Binary(
                            BinOp::Add,
                            Box::new(jsast::id("i")),
                            Box::new(Expr::Num(1.0)),
                        ),
                    ),
                ],
            ),
        ],
    };
    let imm: std::collections::HashSet<String> = ["s".to_owned()].into_iter().collect();
    super::super::post_opt::run_speed_post_opts_with_mode(&mut decl, true, &imm);
    let s = jsast::stmt_to_string(&decl);
    assert!(
        s.contains("$hb_0 = s.buf;") && s.contains("$ho_0 = s.off;"),
        "expected immutable slice buf/off hoisted across call: {s}"
    );
    assert!(
        s.contains("$hb_0[$ho_0 + i].x ="),
        "expected LHS member base replaced with hoisted buf/off: {s}"
    );
}

#[test]
fn hoists_single_use_gzip_sync_slice_props_and_smi_izes_step() {
    let mut decl = Stmt::FunctionDecl {
        name: "gzip_sync".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("k".to_owned(), None),
                ("g".to_owned(), None),
                ("f".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("k"), jsast::member(jsast::id("a"), "len")),
            jsast::assign_stmt(jsast::id("g"), Expr::Num(0.0)),
            jsast::assign_stmt(jsast::id("f"), Expr::Num(4294967295.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("g")),
                    Box::new(jsast::id("k")),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("f"),
                        Expr::Binary(
                            BinOp::BitXor,
                            Box::new(jsast::index(
                                jsast::id("matrix_fflate$tables$CRCT"),
                                Expr::Binary(
                                    BinOp::UShr,
                                    Box::new(Expr::Binary(
                                        BinOp::BitXor,
                                        Box::new(Expr::Binary(
                                            BinOp::BitAnd,
                                            Box::new(jsast::id("f")),
                                            Box::new(Expr::Num(255.0)),
                                        )),
                                        Box::new(jsast::index(
                                            jsast::member(jsast::id("a"), "buf"),
                                            Expr::Binary(
                                                BinOp::Add,
                                                Box::new(jsast::member(jsast::id("a"), "off")),
                                                Box::new(jsast::id("g")),
                                            ),
                                        )),
                                    )),
                                    Box::new(Expr::Num(0.0)),
                                ),
                            )),
                            Box::new(Expr::Binary(
                                BinOp::UShr,
                                Box::new(jsast::id("f")),
                                Box::new(Expr::Num(8.0)),
                            )),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("g"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("g")),
                                Box::new(Expr::Num(1.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                ],
            ),
            Stmt::Return(Some(jsast::id("f"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(
        s.contains("$hb_0 = a.buf;") && s.contains("$ho_0 = a.off;"),
        "expected single-use a.buf and a.off hoisted out of gzip_sync loop: {s}"
    );
    assert!(
        s.contains("$hb_0[$ho_0 + g]"),
        "expected hoisted buf/off inside nested CRCT index: {s}"
    );
    assert!(
        s.contains("g = g + 1 | 0;") && !s.contains("g = g + 1 >>> 0;"),
        "expected counted loop step Smi-ized via k = a.len: {s}"
    );
}

#[test]
fn hoists_while_header_len_and_skips_mutated_or_aliased_slice() {
    // Positive: `while (g < a.len)` with no prior deref hoists `.len`, `.buf`, `.off`.
    let mut decl = Stmt::FunctionDecl {
        name: "sum_bytes".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            jsast::lets(vec![("g".to_owned(), None), ("acc".to_owned(), None)]),
            jsast::assign_stmt(jsast::id("g"), Expr::Num(0.0)),
            jsast::assign_stmt(jsast::id("acc"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("g")),
                    Box::new(jsast::member(jsast::id("a"), "len")),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("acc"),
                        Expr::Binary(
                            BinOp::BitOr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("acc")),
                                Box::new(jsast::index(
                                    jsast::member(jsast::id("a"), "buf"),
                                    Expr::Binary(
                                        BinOp::Add,
                                        Box::new(jsast::member(jsast::id("a"), "off")),
                                        Box::new(jsast::id("g")),
                                    ),
                                )),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("g"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("g")),
                                Box::new(Expr::Num(1.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                ],
            ),
            Stmt::Return(Some(jsast::id("acc"))),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(
        s.contains("$hb_0 = a.buf;")
            && s.contains("$ho_0 = a.off;")
            && s.contains("$hl_0 = a.len;"),
        "expected all three slice props hoisted unconditionally from while header: {s}"
    );
    assert!(
        s.contains("while (g < $hl_0)"),
        "expected while header condition to use hoisted $hl_0: {s}"
    );

    // Negative: aliased slice `b = a` mutating `b.off` inside loop must NOT hoist `a.off`.
    let mut aliased = Stmt::FunctionDecl {
        name: "mut_alias".to_owned(),
        params: vec!["a".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("b".to_owned(), None),
                ("g".to_owned(), None),
                ("acc".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("b"), jsast::id("a")),
            jsast::assign_stmt(jsast::id("g"), Expr::Num(0.0)),
            jsast::assign_stmt(jsast::id("acc"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("g")),
                    Box::new(Expr::Num(4.0)),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("acc"),
                        Expr::Binary(
                            BinOp::Add,
                            Box::new(jsast::id("acc")),
                            Box::new(jsast::index(
                                jsast::member(jsast::id("a"), "buf"),
                                jsast::member(jsast::id("a"), "off"),
                            )),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::member(jsast::id("b"), "off"),
                        Expr::Binary(
                            BinOp::Add,
                            Box::new(jsast::member(jsast::id("b"), "off")),
                            Box::new(Expr::Num(1.0)),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("g"),
                        Expr::Binary(
                            BinOp::Add,
                            Box::new(jsast::id("g")),
                            Box::new(Expr::Num(1.0)),
                        ),
                    ),
                ],
            ),
            Stmt::Return(Some(jsast::id("acc"))),
        ],
    };
    run_speed_post_opts(&mut aliased);
    let sa = jsast::stmt_to_string(&aliased);
    assert!(
        !sa.contains("$ho_0 = a.off;"),
        "mutated alias b.off must prevent hoisting a.off: {sa}"
    );
}

#[test]
fn smi_izes_counted_loop_steps_and_preserves_unbounded_or_double_step_loops() {
    // Positive: `for (;;)` Range::next pattern with const bound 64.
    let mut bounded = Stmt::FunctionDecl {
        name: "bounded".to_owned(),
        params: vec!["arr".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("i".to_owned(), None),
                ("next".to_owned(), None),
                ("acc".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("i"), Expr::Num(0.0)),
            jsast::assign_stmt(jsast::id("acc"), Expr::Num(0.0)),
            Stmt::LoopForever(
                None,
                vec![Stmt::If(
                    Expr::Binary(
                        BinOp::Lt,
                        Box::new(jsast::id("i")),
                        Box::new(Expr::Num(64.0)),
                    ),
                    vec![
                        jsast::assign_stmt(
                            jsast::id("next"),
                            Expr::Binary(
                                BinOp::UShr,
                                Box::new(Expr::Binary(
                                    BinOp::Add,
                                    Box::new(jsast::id("i")),
                                    Box::new(Expr::Num(1.0)),
                                )),
                                Box::new(Expr::Num(0.0)),
                            ),
                        ),
                        jsast::assign_stmt(
                            jsast::id("acc"),
                            Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("acc")),
                                Box::new(jsast::index(jsast::id("arr"), jsast::id("i"))),
                            ),
                        ),
                        jsast::assign_stmt(jsast::id("i"), jsast::id("next")),
                    ],
                    Some(vec![Stmt::Break(None)]),
                )],
            ),
            Stmt::Return(Some(jsast::id("acc"))),
        ],
    };
    run_speed_post_opts(&mut bounded);
    let sb = jsast::stmt_to_string(&bounded);
    assert!(
        sb.contains("next = i + 1 | 0;") && !sb.contains(">>> 0"),
        "expected Range::next step Smi-ized to | 0: {sb}"
    );

    // Negative 1: arbitrary u32 parameter `n` could be >= 2^31.
    let mut unbounded = Stmt::FunctionDecl {
        name: "unbounded".to_owned(),
        params: vec!["n".to_owned()],
        body: vec![
            jsast::lets(vec![("i".to_owned(), None)]),
            jsast::assign_stmt(jsast::id("i"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("i")),
                    Box::new(jsast::id("n")),
                ),
                vec![jsast::assign_stmt(
                    jsast::id("i"),
                    Expr::Binary(
                        BinOp::UShr,
                        Box::new(Expr::Binary(
                            BinOp::Add,
                            Box::new(jsast::id("i")),
                            Box::new(Expr::Num(1.0)),
                        )),
                        Box::new(Expr::Num(0.0)),
                    ),
                )],
            ),
            Stmt::Return(Some(jsast::id("i"))),
        ],
    };
    run_speed_post_opts(&mut unbounded);
    let su = jsast::stmt_to_string(&unbounded);
    assert!(
        su.contains("i = i + 1 >>> 0;"),
        "unbounded u32 parameter loop must keep >>> 0: {su}"
    );

    // Negative 2: two increments per iteration could overflow 0x7fffffff.
    let mut double_step = Stmt::FunctionDecl {
        name: "double_step".to_owned(),
        params: vec![],
        body: vec![
            jsast::lets(vec![("i".to_owned(), None)]),
            jsast::assign_stmt(jsast::id("i"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("i")),
                    Box::new(Expr::Num(2147483647.0)),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("i"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("i")),
                                Box::new(Expr::Num(1.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("i"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("i")),
                                Box::new(Expr::Num(1.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                ],
            ),
            Stmt::Return(Some(jsast::id("i"))),
        ],
    };
    run_speed_post_opts(&mut double_step);
    let sd = jsast::stmt_to_string(&double_step);
    assert_eq!(
        sd.matches("i = i + 1 >>> 0;").count(),
        2,
        "double-increment loop must keep >>> 0: {sd}"
    );
}

#[test]
fn demotes_compress256_registers_through_local_array_and_guards_escapes() {
    // Models `noble-hashes::compress256`: 64-round loop writing `ad = x + z >>> 0; ac = z + am >>> 0;`
    // and shifting `j,p,q,x,k,r,s,y`, then storing into `aK = [j,p,q,x,k,r,s,y]` and accumulating
    // `a[b + A] = aL + az >>> 0`.
    let mut decl = Stmt::FunctionDecl {
        name: "compress256".to_owned(),
        params: vec!["a".to_owned(), "b".to_owned(), "w".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("j".to_owned(), None),
                ("p".to_owned(), None),
                ("q".to_owned(), None),
                ("x".to_owned(), None),
                ("k".to_owned(), None),
                ("r".to_owned(), None),
                ("s".to_owned(), None),
                ("y".to_owned(), None),
                ("i".to_owned(), None),
                ("z".to_owned(), None),
                ("am".to_owned(), None),
                ("ad".to_owned(), None),
                ("ac".to_owned(), None),
                ("aK".to_owned(), None),
                ("A".to_owned(), None),
                ("aL".to_owned(), None),
                ("az".to_owned(), None),
            ]),
            jsast::assign_stmt(jsast::id("j"), jsast::index(jsast::id("a"), Expr::Num(0.0))),
            jsast::assign_stmt(jsast::id("p"), jsast::index(jsast::id("a"), Expr::Num(1.0))),
            jsast::assign_stmt(jsast::id("q"), jsast::index(jsast::id("a"), Expr::Num(2.0))),
            jsast::assign_stmt(jsast::id("x"), jsast::index(jsast::id("a"), Expr::Num(3.0))),
            jsast::assign_stmt(jsast::id("k"), jsast::index(jsast::id("a"), Expr::Num(4.0))),
            jsast::assign_stmt(jsast::id("r"), jsast::index(jsast::id("a"), Expr::Num(5.0))),
            jsast::assign_stmt(jsast::id("s"), jsast::index(jsast::id("a"), Expr::Num(6.0))),
            jsast::assign_stmt(jsast::id("y"), jsast::index(jsast::id("a"), Expr::Num(7.0))),
            jsast::assign_stmt(jsast::id("i"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("i")),
                    Box::new(Expr::Num(64.0)),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("z"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("y")),
                                Box::new(Expr::Binary(
                                    BinOp::BitXor,
                                    Box::new(jsast::id("k")),
                                    Box::new(jsast::index(jsast::id("w"), jsast::id("i"))),
                                )),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("am"),
                        Expr::Binary(
                            BinOp::BitXor,
                            Box::new(Expr::Binary(
                                BinOp::BitAnd,
                                Box::new(jsast::id("j")),
                                Box::new(jsast::id("p")),
                            )),
                            Box::new(jsast::id("q")),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("ad"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("x")),
                                Box::new(jsast::id("z")),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("ac"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("z")),
                                Box::new(jsast::id("am")),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    jsast::assign_stmt(jsast::id("y"), jsast::id("s")),
                    jsast::assign_stmt(jsast::id("s"), jsast::id("r")),
                    jsast::assign_stmt(jsast::id("r"), jsast::id("k")),
                    jsast::assign_stmt(jsast::id("k"), jsast::id("ad")),
                    jsast::assign_stmt(jsast::id("x"), jsast::id("q")),
                    jsast::assign_stmt(jsast::id("q"), jsast::id("p")),
                    jsast::assign_stmt(jsast::id("p"), jsast::id("j")),
                    jsast::assign_stmt(jsast::id("j"), jsast::id("ac")),
                    jsast::assign_stmt(
                        jsast::id("i"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("i")),
                                Box::new(Expr::Num(1.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                ],
            ),
            jsast::assign_stmt(
                jsast::id("aK"),
                Expr::Array(vec![
                    jsast::id("j"),
                    jsast::id("p"),
                    jsast::id("q"),
                    jsast::id("x"),
                    jsast::id("k"),
                    jsast::id("r"),
                    jsast::id("s"),
                    jsast::id("y"),
                ]),
            ),
            jsast::assign_stmt(jsast::id("A"), Expr::Num(0.0)),
            Stmt::While(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("A")),
                    Box::new(Expr::Num(8.0)),
                ),
                vec![
                    jsast::assign_stmt(
                        jsast::id("aL"),
                        jsast::index(jsast::id("aK"), jsast::id("A")),
                    ),
                    jsast::assign_stmt(
                        jsast::id("az"),
                        jsast::index(
                            jsast::id("a"),
                            Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("b")),
                                Box::new(jsast::id("A")),
                            ),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::index(
                            jsast::id("a"),
                            Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("b")),
                                Box::new(jsast::id("A")),
                            ),
                        ),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("aL")),
                                Box::new(jsast::id("az")),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                    jsast::assign_stmt(
                        jsast::id("A"),
                        Expr::Binary(
                            BinOp::UShr,
                            Box::new(Expr::Binary(
                                BinOp::Add,
                                Box::new(jsast::id("A")),
                                Box::new(Expr::Num(1.0)),
                            )),
                            Box::new(Expr::Num(0.0)),
                        ),
                    ),
                ],
            ),
        ],
    };
    run_speed_post_opts(&mut decl);
    let s = jsast::stmt_to_string(&decl);
    assert!(
        s.contains("ad = x + z | 0;") && s.contains("ac = z + am | 0;"),
        "expected SHA-256 loop-carried registers demoted to signed Int32 | 0: {s}"
    );
    assert!(
        s.contains("a[b] = aL + az >>> 0;") && s.contains("a[b + 7] = aL + az >>> 0;"),
        "expected final state store to preserve >>> 0 canonical unsigned write (with b + 0 folded to b): {s}"
    );

    // Negative: local read by `<`, array index, `===`, or returned must NOT be demoted.
    let mut neg = Stmt::FunctionDecl {
        name: "neg".to_owned(),
        params: vec!["a".to_owned(), "b".to_owned(), "arr".to_owned()],
        body: vec![
            jsast::lets(vec![
                ("v_cmp".to_owned(), None),
                ("v_idx".to_owned(), None),
                ("v_eq".to_owned(), None),
                ("v_ret".to_owned(), None),
            ]),
            jsast::assign_stmt(
                jsast::id("v_cmp"),
                Expr::Binary(
                    BinOp::UShr,
                    Box::new(Expr::Binary(
                        BinOp::Add,
                        Box::new(jsast::id("a")),
                        Box::new(jsast::id("b")),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            jsast::assign_stmt(
                jsast::id("v_idx"),
                Expr::Binary(
                    BinOp::UShr,
                    Box::new(Expr::Binary(
                        BinOp::Add,
                        Box::new(jsast::id("a")),
                        Box::new(jsast::id("b")),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            jsast::assign_stmt(
                jsast::id("v_eq"),
                Expr::Binary(
                    BinOp::UShr,
                    Box::new(Expr::Binary(
                        BinOp::Add,
                        Box::new(jsast::id("a")),
                        Box::new(jsast::id("b")),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            jsast::assign_stmt(
                jsast::id("v_ret"),
                Expr::Binary(
                    BinOp::UShr,
                    Box::new(Expr::Binary(
                        BinOp::Add,
                        Box::new(jsast::id("a")),
                        Box::new(jsast::id("b")),
                    )),
                    Box::new(Expr::Num(0.0)),
                ),
            ),
            Stmt::If(
                Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id("v_cmp")),
                    Box::new(Expr::Num(10.0)),
                ),
                vec![Stmt::Return(Some(jsast::index(
                    jsast::id("arr"),
                    jsast::id("v_idx"),
                )))],
                None,
            ),
            Stmt::If(
                Expr::Binary(
                    BinOp::StrictEq,
                    Box::new(jsast::id("v_eq")),
                    Box::new(Expr::Num(4294967295.0)),
                ),
                vec![Stmt::Return(Some(Expr::Num(1.0)))],
                None,
            ),
            Stmt::Return(Some(jsast::id("v_ret"))),
        ],
    };
    run_speed_post_opts(&mut neg);
    let sn = jsast::stmt_to_string(&neg);
    assert_eq!(
        sn.matches("a + b >>> 0").count(),
        4,
        "all four non-bit32-consumed locals must preserve >>> 0: {sn}"
    );
}

#[test]
fn dominated_guard_eliminates_cast_wrapped_and_seq_checks_and_respects_reassignment() {
    let panic_throw = |idx: &str, len: &str| {
        Stmt::Throw(Expr::Call(
            Box::new(jsast::id("core$panicking$panic_bounds_check$h1")),
            vec![jsast::id(idx), jsast::id(len), jsast::id("$hc_1")],
        ))
    };
    let bounds_if = |idx: &str, len: &str| {
        Stmt::If(
            Expr::Unary(
                UnOp::Not,
                Box::new(Expr::Binary(
                    BinOp::Lt,
                    Box::new(jsast::id(idx)),
                    Box::new(jsast::id(len)),
                )),
            ),
            vec![panic_throw(idx, len)],
            None,
        )
    };
    let ushr0 = |name: &str| {
        Expr::Binary(
            BinOp::UShr,
            Box::new(jsast::id(name)),
            Box::new(Expr::Num(0.0)),
        )
    };

    // Positive 1: `t = g >>> 0; if (!(t < c)) throw ...; a[t].prev = 1; j = g >>> 0; if (!(j < c)) throw ...;`
    let mut stmts = vec![
        jsast::assign_stmt(jsast::id("t"), ushr0("g")),
        bounds_if("t", "c"),
        jsast::assign_stmt(
            Expr::Member(
                Box::new(jsast::index(jsast::id("a"), jsast::id("t"))),
                "prev".to_owned(),
            ),
            Expr::Num(1.0),
        ),
        jsast::assign_stmt(jsast::id("j"), ushr0("g")),
        bounds_if("j", "c"),
    ];
    super::eliminate_dominated_guards(&mut stmts);
    let out: String = stmts.iter().map(jsast::stmt_to_string).collect();
    assert_eq!(
        out.matches("panic_bounds_check").count(),
        1,
        "cast-wrapped duplicate check across field store must be eliminated: {out}"
    );

    // Positive 2: `Expr::Seq` slice unpacking `dL = c9, dM = cu` propagates length alias!
    let mut seq_stmts = vec![
        Stmt::ExprStmt(Expr::Seq(vec![
            Expr::Assign(Box::new(jsast::id("dL")), Box::new(jsast::id("c9"))),
            Expr::Assign(Box::new(jsast::id("dM")), Box::new(jsast::id("cu"))),
        ])),
        bounds_if("x", "dM"),
        bounds_if("x", "cu"),
    ];
    super::eliminate_dominated_guards(&mut seq_stmts);
    let seq_out: String = seq_stmts.iter().map(jsast::stmt_to_string).collect();
    assert_eq!(
        seq_out.matches("panic_bounds_check").count(),
        1,
        "Expr::Seq alias `dM = cu` must allow eliminating dominated `x < cu` check: {seq_out}"
    );

    // Negative 1: reassigning `g` between checks MUST preserve the second check!
    let mut neg_g = vec![
        jsast::assign_stmt(jsast::id("t"), ushr0("g")),
        bounds_if("t", "c"),
        jsast::assign_stmt(
            jsast::id("g"),
            Expr::Binary(
                BinOp::Add,
                Box::new(jsast::id("g")),
                Box::new(Expr::Num(1.0)),
            ),
        ),
        jsast::assign_stmt(jsast::id("j"), ushr0("g")),
        bounds_if("j", "c"),
    ];
    super::eliminate_dominated_guards(&mut neg_g);
    let ng_out: String = neg_g.iter().map(jsast::stmt_to_string).collect();
    assert_eq!(
        ng_out.matches("panic_bounds_check").count(),
        2,
        "reassigning `g` must invalidate dominated check: {ng_out}"
    );

    // Negative 2: reassigning `c` between checks MUST preserve the second check!
    let mut neg_c = vec![
        jsast::assign_stmt(jsast::id("t"), ushr0("g")),
        bounds_if("t", "c"),
        jsast::assign_stmt(jsast::id("c"), jsast::id("other")),
        jsast::assign_stmt(jsast::id("j"), ushr0("g")),
        bounds_if("j", "c"),
    ];
    super::eliminate_dominated_guards(&mut neg_c);
    let nc_out: String = neg_c.iter().map(jsast::stmt_to_string).collect();
    assert_eq!(
        nc_out.matches("panic_bounds_check").count(),
        2,
        "reassigning `c` must invalidate dominated check: {nc_out}"
    );
}
