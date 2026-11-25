//! ES2018 BigInt constants: one module-level binding per distinct value.
//!
//! `js-ecma-target=2018` has no BigInt literal, so the printer spells `Expr::BigInt` as a call,
//! `BigInt("33")` (`jsast.rs`, `Printer::expr_bare`), and the engine parses that string again
//! every time the expression runs. In a 64-bit arithmetic loop that parse is most of the cost. The
//! JIR lane therefore spells each constant of a lowered function as an identifier ([`hoist`]),
//! and the lane driver adds one module-level `const <name>=<ok>&&BigInt("<digits>")` per distinct
//! value the printed items name ([`BigIntNames::item`]), plus the two items those need
//! ([`BigIntNames::support`]), exactly as it adds the flat-ABI spill
//! registers: the names are prepared in the driver's `defined`/`live` sets before the lowerings
//! are checked against them, and the items enter the table after phase C, so the reachability walk,
//! `order`, chunking and the item rename treat them like any other binding.
//!
//! # What is rewritten
//!
//! Only `Expr::BigInt` nodes of a JIR-lane function declaration (`js::function::lower`, and a
//! committed native body), in the final (phase C) lowering, on the ES2018 target. A function body
//! is what runs more than once. A `static` or vtable initializer runs exactly once, while the
//! module evaluates, so it keeps `BigInt("…")`: naming its constants would cost a declaration per
//! value and save nothing (`core`'s float formatting tables alone hold 1,302 constants, 1,271 of
//! them used once). That also means no module-level initializer ever reads one of these bindings,
//! so their position among the bindings `order` lays out cannot matter. The baseline lane's printed
//! text keeps `BigInt("…")` too: it is never re-parsed. `js-ecma-target=latest` prints `33n`,
//! which needs no hoisting, and never reaches this module.
//!
//! # Engines without `BigInt`
//!
//! `BigInt` is a runtime requirement of 64-bit code, not of a program: a program whose 32-bit paths
//! never compute with a 64-bit value loads and runs on an engine without it (Safari 12.1–13.1,
//! Firefox 65–67), and a function that does reach a 64-bit constant there throws `ReferenceError:
//! BigInt is not defined` where it evaluates `BigInt`. The hoist keeps both:
//!
//! * nothing calls `BigInt` while the program loads. `<ok>` is `typeof BigInt==="function"`, and a
//!   binding is `<ok>&&BigInt("<digits>")`, so without `BigInt` every binding holds `false`;
//! * no function reads a binding that holds `false`. The first constant an activation can evaluate
//!   on each path is spelled `(<ok>?<name>:<no>())`, and `<no>` is `function(){throw BigInt()}`,
//!   which never returns. On an engine without `BigInt` it throws the `ReferenceError: BigInt is
//!   not defined` the inline `BigInt("…")` threw, at the same point in the evaluation. A constant
//!   that every path reaches only after a guard was evaluated is spelled bare, because a guard that
//!   did not throw means `<ok>` is true and every binding holds its value ([`hoist`] tracks this
//!   in evaluation order, conservatively).
//!
//! `<ok>` is read once, while the program loads, so what the host does to `BigInt` afterwards does
//! not bring the constants back. A program that loaded without a `BigInt` function keeps every
//! binding at `false`, and every guarded read keeps throwing: `BigInt()` is called each time, and
//! it either throws (`ReferenceError` while `BigInt` is still missing, `TypeError` from a real
//! `BigInt` or from a global that is not a function) or returns, and then `<no>` throws the value
//! it returned (`globalThis.BigInt = Number` makes every guarded read throw `0`). No guarded read
//! of such a program yields a value. A program that loaded with `BigInt` keeps the values it
//! computed then, whatever `BigInt` becomes.
//!
//! In a loop the guard is the first constant of the body, once per iteration: the block loop of
//! the `murmur3` kernel reads constants eight times per iteration and guards the first read.
//!
//! # Trial lowerings see inline constants
//!
//! The whole-program cost trials (`physical_program_metrics` and the representation planner's
//! `program_cost`) lower through `LowerInput { bigints: None, .. }`: they price, and the
//! lowering-reuse cache (`plan/lowering_reuse.rs`) stores, the inline spelling the unmodified
//! linker prices. The hoist is a pure function of the final declaration applied after every
//! decision is made, so no decision can depend on it, and no cached trial lowering is ever
//! installed in the program: phase C lowers every item afresh.
//!
//! # Names
//!
//! A constant's name is `<prefix><digits>`, a minus sign spelled `m`: the same value has the same
//! name in every item, so two bodies that printed the same text before the rewrite print the same
//! text after it, which is what `link_dedup` compares. The prefix is chosen once per link step
//! (`$BI_`, else `$BI$1_`, `$BI$2_`, ...) so that no item name, no reference, no printed item text
//! and not the runtime shim already contain it. The lowerer's own names (`$vN`, `$tN`, `$rN`, `$b`,
//! `$cN`, `$sN`, `$native…`, `$invoke…`, `$loc`, …) and every name a codegen-minified local can take
//! (`a`, `b`, …, never `$`) are lowercase or `$`-free by construction, so none of them can be one;
//! under `js-minify=on` the item rename (`minify::item_renames`) shortens the constants like any
//! other renamable item.
//!
//! # Why sharing one binding is unobservable
//!
//! A BigInt is an immutable primitive compared by value (`===`, `Object.is`, `typeof`), it cannot
//! key a `WeakMap` or carry properties, and the binding is `const`. What changes is when
//! `BigInt("…")` runs: once, while the module initializes, instead of at each use. That is
//! observable only by a program that replaces `globalThis.BigInt` after this module was evaluated
//! (the hoisted value is the original `BigInt`'s; one that defines `BigInt` only later finds every
//! guard throwing, see "Engines without `BigInt`"), or one that calls a function using a constant
//! while module initialization has not yet reached the constant's declaration (a temporal dead
//! zone `ReferenceError`). Neither happens here: the program and the shim never assign `BigInt`,
//! and module evaluation runs binding initializers only, which are literals that call no program
//! function and name no constant; every function runs after the module (and, split, every module
//! of its chunk graph) has evaluated.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{item_map, ItemKind, JsItem, JsName, LinkItem, Linkage};
use crate::jsast::{self, ArrowBody, BinOp, Expr, Stmt, UnOp};

/// The spelling every candidate prefix starts from.
const BASE: &str = "$BI";

/// How many prefixes are tried before the program is left with inline constants.
const CANDIDATES: usize = 64;

/// The names one link step gives its BigInt constants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BigIntNames {
    prefix: String,
}

impl BigIntNames {
    /// A prefix nothing in the program spells: no key of `table`, no item's references, no item's
    /// printed text, none of `extra` (the spill registers, which are not in the table yet) and not
    /// the runtime shim contain it. `None` when every candidate is taken, which leaves the program
    /// on inline constants.
    pub fn allocate(table: &BTreeMap<JsName, LinkItem>, extra: &[JsName]) -> Option<BigIntNames> {
        let spelled = |prefix: &str| {
            crate::plan::cost::SHIM.contains(prefix)
                || extra.iter().any(|name| name.as_str().contains(prefix))
                || table.iter().any(|(name, item)| {
                    name.as_str().contains(prefix)
                        || item.js.contains(prefix)
                        || item.refs.iter().any(|name| name.as_str().contains(prefix))
                })
        };
        std::iter::once(format!("{BASE}_"))
            .chain((1..CANDIDATES).map(|suffix| format!("{BASE}${suffix}_")))
            .find(|prefix| !spelled(prefix))
            .map(|prefix| BigIntNames { prefix })
    }

    /// The binding that holds `digits`, or `None` when `digits` is not a canonical decimal integer
    /// (`0`, or an optional `-` and a digit string without a leading zero), which stays inline.
    pub fn name(&self, digits: &str) -> Option<String> {
        let (negative, magnitude) = match digits.strip_prefix('-') {
            Some(magnitude) => (true, magnitude),
            None => (false, digits),
        };
        let canonical = !magnitude.is_empty()
            && magnitude.bytes().all(|byte| byte.is_ascii_digit())
            && (magnitude == "0" || !magnitude.starts_with('0'))
            && !(negative && magnitude == "0");
        canonical.then(|| {
            format!(
                "{}{}{magnitude}",
                self.prefix,
                if negative { "m" } else { "" }
            )
        })
    }

    /// The binding that says whether the engine has a `BigInt` function: `const <ok>=typeof
    /// BigInt==="function"`. Every constant's initializer and every guarded use reads it.
    pub fn ok(&self) -> String {
        format!("{}ok", self.prefix)
    }

    /// The function a guarded use calls when [`BigIntNames::ok`] is false: `function <no>(){throw
    /// BigInt()}`, which never returns: it throws the `ReferenceError` a `BigInt("…")` call throws
    /// on an engine without `BigInt`, whatever `BigInt()` throws where a `BigInt` exists, and the
    /// value `BigInt()` returned where a stand-in defined after load returns one.
    pub fn no(&self) -> String {
        format!("{}no", self.prefix)
    }

    /// The module-level binding of `digits`: `const <name>=<ok>&&BigInt("<digits>")`, which calls
    /// `BigInt` only on an engine that has it. `None` for digits [`BigIntNames::name`] leaves
    /// inline.
    pub fn item(&self, digits: &str) -> Option<LinkItem> {
        let name = self.name(digits)?;
        let init = jsast::binary(
            BinOp::And,
            jsast::id(self.ok()),
            Expr::BigInt(digits.to_owned()),
        );
        Some(link_item(
            &name,
            ItemKind::Const,
            jsast::const_(name.as_str(), init),
            [self.ok()],
            format!("BigInt constant {digits}"),
        ))
    }

    /// The two items every program with a constant needs besides the constants: the
    /// [`BigIntNames::ok`] binding and the [`BigIntNames::no`] function.
    pub fn support(&self) -> [LinkItem; 2] {
        let ok = self.ok();
        let no = self.no();
        let has_bigint = jsast::binary(
            BinOp::StrictEq,
            jsast::unary(UnOp::TypeOf, jsast::id("BigInt")),
            jsast::string("function"),
        );
        let throws = vec![Stmt::Throw(jsast::call(jsast::id("BigInt"), Vec::new()))];
        [
            link_item(
                &ok,
                ItemKind::Const,
                jsast::const_(ok.as_str(), has_bigint),
                [],
                "whether the engine has BigInt".to_owned(),
            ),
            link_item(
                &no,
                ItemKind::Fn,
                jsast::function(no.as_str(), Vec::new(), throws),
                [],
                "a BigInt constant on an engine without BigInt".to_owned(),
            ),
        ]
    }
}

/// A link-generated item around `decl`, printed the way the driver prints a lowered item
/// (`JsItem::to_js_mapped`), so the build's style and target apply.
fn link_item<const N: usize>(
    name: &str,
    kind: ItemKind,
    decl: Stmt,
    refs: [String; N],
    debug_path: String,
) -> LinkItem {
    let name = JsName::new(name.to_owned());
    let refs: BTreeSet<JsName> = refs.into_iter().map(JsName::new).collect();
    let printed = JsItem {
        name: name.clone(),
        kind,
        decl,
        refs: refs.clone(),
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path: debug_path.clone(),
        order: None,
        loc: None,
        names: Vec::new(),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: None,
        jir_decline: None,
        jir_item: None,
    };
    let (js, mappings) = printed.to_js_mapped(crate::opts::get().comments);
    LinkItem {
        name,
        kind,
        js,
        refs,
        zombies: Vec::new(),
        linkage: Linkage::internal(),
        debug_path,
        crate_name: "generated".to_owned(),
        order: None,
        map: item_map(mappings),
        dts: None,
        optimization: Default::default(),
        allocator_provider: Default::default(),
        jir: Default::default(),
    }
}

/// Spells every BigInt constant of the function `decl` as its binding's name, and adds the digits
/// of each one it names to `used`. A constant [`BigIntNames::name`] refuses keeps its inline
/// spelling. Verbatim JavaScript (`Expr::Raw`, `Stmt::Raw`) is opaque and left alone.
///
/// The first constant an activation of the function can evaluate on each path is guarded,
/// `(<ok>?<name>:<no>())`, which throws exactly where the inline `BigInt("…")` would have thrown on
/// an engine without `BigInt`. A constant that every path reaches only after a guard has been
/// evaluated is spelled bare: once one guard has run without throwing, the engine has `BigInt` and
/// every binding holds its value. See the module documentation, "Engines without `BigInt`".
pub fn hoist(decl: &mut Stmt, names: &BigIntNames, used: &mut BTreeSet<String>) {
    Hoist { names, used }.stmt(decl, false);
}

struct Hoist<'a> {
    names: &'a BigIntNames,
    used: &'a mut BTreeSet<String>,
}

/// The walk threads one fact through JavaScript's evaluation order: `guarded`, whether a guard has
/// certainly been evaluated in this activation before the current point. Each method takes the
/// fact on entry and returns it on exit. A construct that may skip part of itself (a branch, a
/// case, a loop body, the right of `&&` and `||`, the arms of `?:`, a `try` body, a labeled
/// statement a `break` may leave) passes the entry fact into that part and returns the fact it had
/// before it; an arrow's body is another activation and starts unguarded.
impl Hoist<'_> {
    fn stmts(&mut self, stmts: &mut [Stmt], mut guarded: bool) -> bool {
        for stmt in stmts {
            guarded = self.stmt(stmt, guarded);
        }
        guarded
    }

    fn stmt(&mut self, stmt: &mut Stmt, guarded: bool) -> bool {
        match stmt {
            Stmt::ExprStmt(expr) | Stmt::Throw(expr) | Stmt::Const(_, expr) => {
                self.expr(expr, guarded)
            }
            Stmt::Let(_, init) | Stmt::Return(init) => match init {
                Some(init) => self.expr(init, guarded),
                None => guarded,
            },
            Stmt::Lets(declarations) => {
                let mut guarded = guarded;
                for (_, init) in declarations {
                    if let Some(init) = init {
                        guarded = self.expr(init, guarded);
                    }
                }
                guarded
            }
            Stmt::If(test, then, els) => {
                let guarded = self.expr(test, guarded);
                self.stmts(then, guarded);
                if let Some(els) = els {
                    self.stmts(els, guarded);
                }
                guarded
            }
            Stmt::Switch(discriminant, cases) => {
                let guarded = self.expr(discriminant, guarded);
                for case in cases {
                    for test in &mut case.tests {
                        self.expr(test, guarded);
                    }
                    self.stmts(&mut case.body, guarded);
                }
                guarded
            }
            Stmt::Labeled(_, inner) => {
                self.stmt(inner, guarded);
                guarded
            }
            Stmt::Block(body) => self.stmts(body, guarded),
            Stmt::LoopForever(_, body) => {
                self.stmts(body, guarded);
                guarded
            }
            // The test runs before the body, and before the loop is left, at least once.
            Stmt::While(test, body) => {
                let guarded = self.expr(test, guarded);
                self.stmts(body, guarded);
                guarded
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.stmts(body, guarded);
                self.stmts(handler, guarded);
                guarded
            }
            Stmt::FunctionDecl { body, .. } => {
                self.stmts(body, false);
                guarded
            }
            Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => guarded,
        }
    }

    fn expr(&mut self, expr: &mut Expr, guarded: bool) -> bool {
        match expr {
            Expr::BigInt(digits) => {
                let Some(name) = self.names.name(digits) else {
                    return guarded;
                };
                self.used.insert(std::mem::take(digits));
                *expr = if guarded {
                    Expr::Ident(name)
                } else {
                    jsast::cond(
                        jsast::id(self.names.ok()),
                        Expr::Ident(name),
                        jsast::call(jsast::id(self.names.no()), Vec::new()),
                    )
                };
                true
            }
            Expr::Member(object, _) | Expr::Unary(_, object) => self.expr(object, guarded),
            Expr::Binary(BinOp::And | BinOp::Or, left, right) => {
                let guarded = self.expr(left, guarded);
                self.expr(right, guarded);
                guarded
            }
            // An assignment evaluates its target's object and key before its value.
            Expr::Index(left, right)
            | Expr::Binary(_, left, right)
            | Expr::Assign(left, right)
            | Expr::CompoundAssign(_, left, right) => {
                let guarded = self.expr(left, guarded);
                self.expr(right, guarded)
            }
            Expr::Call(callee, arguments) | Expr::New(callee, arguments) => {
                let mut guarded = self.expr(callee, guarded);
                for argument in arguments {
                    guarded = self.expr(argument, guarded);
                }
                guarded
            }
            Expr::Cond(test, consequent, alternate) => {
                let guarded = self.expr(test, guarded);
                self.expr(consequent, guarded);
                self.expr(alternate, guarded);
                guarded
            }
            Expr::Object(entries) => {
                let mut guarded = guarded;
                for (_, value) in entries {
                    guarded = self.expr(value, guarded);
                }
                guarded
            }
            Expr::Array(items) | Expr::Seq(items) => {
                let mut guarded = guarded;
                for item in items {
                    guarded = self.expr(item, guarded);
                }
                guarded
            }
            Expr::Arrow(_, body) => {
                match body {
                    ArrowBody::Expr(body) => {
                        self.expr(body, false);
                    }
                    ArrowBody::Block(body) => {
                        self.stmts(body, false);
                    }
                }
                guarded
            }
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::Ident(_)
            | Expr::Raw(_) => guarded,
        }
    }
}

/// The integer a number literal spells exactly, which `BigInt(<literal>)` widens without loss: a
/// finite, integral `Expr::Num` within ±2^53. `None` for anything else.
pub fn integer_literal(expr: &Expr) -> Option<i64> {
    const SAFE: f64 = 9_007_199_254_740_992.0;
    match expr {
        Expr::Num(value) if value.is_finite() && value.fract() == 0.0 && value.abs() <= SAFE => {
            Some(*value as i64)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsast::BinOp;

    fn names() -> BigIntNames {
        BigIntNames {
            prefix: "$BI_".to_owned(),
        }
    }

    fn item(name: &str, js: &str, refs: &[&str]) -> (JsName, LinkItem) {
        let mut item = names().item("1").expect("a canonical literal");
        item.name = JsName::new(name.to_owned());
        item.js = js.to_owned();
        item.refs = refs
            .iter()
            .map(|name| JsName::new((*name).to_owned()))
            .collect();
        (item.name.clone(), item)
    }

    #[test]
    fn a_name_is_the_value_and_only_canonical_decimals_get_one() {
        let names = names();
        assert_eq!(names.name("0").as_deref(), Some("$BI_0"));
        assert_eq!(names.name("33").as_deref(), Some("$BI_33"));
        assert_eq!(
            names.name("-9223372036854775808").as_deref(),
            Some("$BI_m9223372036854775808")
        );
        assert_eq!(
            names
                .name("340282366920938463463374607431768211455")
                .as_deref(),
            Some("$BI_340282366920938463463374607431768211455")
        );
        for refused in ["", "-", "-0", "007", "0x1f", "1e3", "1.5", "+1", " 1"] {
            assert_eq!(names.name(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn the_prefix_steps_past_anything_the_program_already_spells() {
        let empty = BTreeMap::new();
        assert_eq!(BigIntNames::allocate(&empty, &[]), Some(names()));

        for table in [
            BTreeMap::from([item("$BI_7", "function $BI_7(){}", &[])]),
            BTreeMap::from([item("f", "function f(){return g()}", &["$BI_7"])]),
            BTreeMap::from([item("f", "function f(){let $BI_7=1;return $BI_7}", &[])]),
        ] {
            let allocated = BigIntNames::allocate(&table, &[]).expect("a free prefix");
            assert_eq!(allocated.prefix, "$BI$1_");
            assert_eq!(allocated.name("7").as_deref(), Some("$BI$1_7"));
        }
        let spill = [JsName::new("$BI_1".to_owned())];
        assert_eq!(
            BigIntNames::allocate(&empty, &spill).map(|names| names.prefix),
            Some("$BI$1_".to_owned())
        );
    }

    #[test]
    fn every_bigint_node_is_named_and_raw_text_is_left_alone() {
        let mut decl = jsast::function(
            "f",
            vec!["a".to_owned()],
            vec![
                jsast::assign_stmt(
                    jsast::id("a"),
                    jsast::binary(
                        BinOp::Shr,
                        jsast::id("a"),
                        jsast::binary(BinOp::BitAnd, jsast::bigint(33u32), jsast::bigint(63u32)),
                    ),
                ),
                Stmt::Return(Some(jsast::array(vec![
                    jsast::arrow(vec!["b".to_owned()], jsast::bigint(-1i64)),
                    jsast::bigint(33u32),
                    Expr::BigInt("007".to_owned()),
                    Expr::Raw("BigInt(\"5\")".to_owned()),
                ]))),
            ],
        );
        let mut used = BTreeSet::new();
        hoist(&mut decl, &names(), &mut used);
        assert_eq!(
            used,
            BTreeSet::from(["-1".to_owned(), "33".to_owned(), "63".to_owned()])
        );
        let printed = jsast::stmt_to_string_styled_for_target(
            &decl,
            jsast::Style::Compact,
            crate::opts::EcmaTarget::Es2018,
        );
        // The first constant is guarded and everything after it is bare; the arrow is another
        // activation, so its first constant is guarded again.
        assert_eq!(
            printed,
            "function f(a){a=a>>(($BI_ok?$BI_33:$BI_no())&$BI_63);\
             return[b=>$BI_ok?$BI_m1:$BI_no(),$BI_33,BigInt(\"007\"),(BigInt(\"5\"))]}"
        );
    }

    fn hoisted(body: Vec<Stmt>) -> String {
        let mut decl = jsast::function("f", vec!["a".to_owned()], body);
        hoist(&mut decl, &names(), &mut BTreeSet::new());
        jsast::stmt_to_string_styled_for_target(
            &decl,
            jsast::Style::Compact,
            crate::opts::EcmaTarget::Es2018,
        )
    }

    fn uses(left: u32, right: u32) -> Expr {
        jsast::binary(BinOp::Add, jsast::bigint(left), jsast::bigint(right))
    }

    /// A guard inside a part of a construct that may not run covers nothing after the construct;
    /// one in a part that always runs first does.
    #[test]
    fn only_a_guard_that_has_certainly_run_covers_a_later_constant() {
        let ret = |e: Expr| Stmt::Return(Some(e));
        let expr = |e: Expr| Stmt::ExprStmt(e);
        let guard = |n: u32| format!("$BI_ok?$BI_{n}:$BI_no()");

        // A branch, a loop body, a labeled block, a `try` body: the constant after them is guarded.
        for (construct, printed) in [
            (
                Stmt::If(jsast::id("a"), vec![expr(jsast::bigint(1u32))], None),
                "if(a)",
            ),
            (
                Stmt::LoopForever(None, vec![expr(jsast::bigint(1u32)), Stmt::Break(None)]),
                "for(;;)",
            ),
            (
                Stmt::Labeled(
                    "l".to_owned(),
                    Box::new(Stmt::Block(vec![expr(jsast::bigint(1u32))])),
                ),
                "l:",
            ),
            (
                Stmt::TryCatch {
                    body: vec![expr(jsast::bigint(1u32))],
                    binding: "e".to_owned(),
                    handler: Vec::new(),
                },
                "try",
            ),
        ] {
            let js = hoisted(vec![construct, ret(jsast::bigint(2u32))]);
            assert!(
                js.contains(&guard(1)) && js.contains(&guard(2)),
                "{printed}: {js}"
            );
        }

        // The right of `&&`, the arms of `?:`: not certain. Their left, their test: certain.
        let js = hoisted(vec![
            expr(jsast::binary(
                BinOp::And,
                jsast::id("a"),
                jsast::bigint(1u32),
            )),
            expr(jsast::cond(
                jsast::id("a"),
                jsast::bigint(1u32),
                jsast::bigint(1u32),
            )),
            ret(jsast::bigint(2u32)),
        ]);
        assert_eq!(js.matches(&guard(1)).count(), 3, "{js}");
        assert!(js.contains(&guard(2)), "{js}");
        let js = hoisted(vec![
            expr(jsast::binary(
                BinOp::And,
                jsast::bigint(1u32),
                jsast::id("a"),
            )),
            ret(jsast::cond(
                jsast::bigint(2u32),
                jsast::bigint(3u32),
                jsast::id("a"),
            )),
        ]);
        assert_eq!(
            js,
            format!("function f(a){{({})&&a;return $BI_2?$BI_3:a}}", guard(1))
        );

        // Statements in order, a `while` test, a switch discriminant, a block: certain.
        let js = hoisted(vec![
            Stmt::While(uses(1, 2), vec![expr(jsast::bigint(3u32))]),
            Stmt::Switch(
                jsast::bigint(4u32),
                vec![jsast::SwitchCase {
                    tests: vec![jsast::bigint(5u32)],
                    body: vec![expr(jsast::bigint(6u32))],
                }],
            ),
            Stmt::Block(vec![expr(jsast::bigint(7u32))]),
            ret(jsast::bigint(8u32)),
        ]);
        assert_eq!(js.matches("$BI_no()").count(), 1, "{js}");
        assert!(
            js.starts_with(&format!("function f(a){{while(({})+$BI_2)", guard(1))),
            "{js}"
        );

        // An arrow's body is another activation; creating the arrow guards nothing.
        let js = hoisted(vec![
            expr(jsast::arrow(Vec::new(), jsast::bigint(1u32))),
            ret(jsast::bigint(1u32)),
        ]);
        assert_eq!(js.matches(&guard(1)).count(), 2, "{js}");
    }

    /// The bindings call `BigInt` only on an engine that has it, and the guard's other arm always
    /// throws.
    #[test]
    fn the_bindings_and_the_guard_never_call_bigint_on_an_engine_without_it() {
        let names = names();
        let item = names.item("33").expect("a canonical literal");
        // The unit tests print for `js-ecma-target=latest`, where the literal is `33n`.
        assert_eq!(item.js.trim(), "const $BI_33 = $BI_ok && 33n;");
        assert_eq!(
            item.refs,
            BTreeSet::from([JsName::new("$BI_ok".to_owned())])
        );
        let [ok, no] = names.support();
        assert_eq!(
            ok.js.trim(),
            "const $BI_ok = typeof BigInt === \"function\";"
        );
        assert_eq!(ok.kind, ItemKind::Const);
        assert!(
            no.js
                .trim_end()
                .ends_with("function $BI_no() {\n  throw BigInt();\n}"),
            "{}",
            no.js
        );
        assert_eq!(no.kind, ItemKind::Fn);
    }

    #[test]
    fn only_exact_integer_numbers_are_widened_at_compile_time() {
        assert_eq!(integer_literal(&jsast::num(33)), Some(33));
        assert_eq!(integer_literal(&jsast::num(-0.0)), Some(0));
        assert_eq!(
            integer_literal(&jsast::num(9_007_199_254_740_992.0)),
            Some(9_007_199_254_740_992)
        );
        for refused in [
            jsast::num(1.5),
            jsast::num(f64::NAN),
            jsast::num(f64::INFINITY),
            jsast::num(18_014_398_509_481_984.0),
            jsast::id("a"),
            jsast::raw_num("33"),
        ] {
            assert_eq!(integer_literal(&refused), None, "{refused:?}");
        }
    }
}
