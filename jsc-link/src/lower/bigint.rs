//! ES2018 BigInt constants: one module-level binding per distinct value.
//!
//! `js-ecma-target=2018` has no BigInt literal, so the printer spells `Expr::BigInt` as a call,
//! `BigInt("33")` (`jsast.rs`, `Printer::expr_bare`), and the engine parses that string again
//! every time the expression runs. In a 64-bit arithmetic loop that parse is most of the cost. The
//! JIR lane therefore spells each constant of a lowered function as an identifier ([`hoist`]),
//! and the lane driver adds one module-level `const <name>=BigInt("<digits>")` per distinct value
//! the printed items name ([`BigIntNames::item`]), exactly as it adds the flat-ABI spill
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
//! (the hoisted value is the original `BigInt`'s), or one that calls a function using a constant
//! while module initialization has not yet reached the constant's declaration (a temporal dead
//! zone `ReferenceError`). Neither happens here: the program and the shim never assign `BigInt`,
//! and module evaluation runs binding initializers only, which are literals that call no program
//! function and name no constant; every function runs after the module (and, split, every module
//! of its chunk graph) has evaluated.

use std::collections::{BTreeMap, BTreeSet};

use crate::item::{item_map, ItemKind, JsItem, JsName, LinkItem, Linkage};
use crate::jsast::{self, ArrowBody, Expr, Stmt};

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

    /// The module-level binding of `digits`: `const <name>=BigInt("<digits>")`, printed the way the
    /// driver prints a lowered item (`JsItem::to_js_mapped`), so the build's style and target
    /// apply. `None` for digits [`BigIntNames::name`] leaves inline.
    pub fn item(&self, digits: &str) -> Option<LinkItem> {
        let name = JsName::new(self.name(digits)?);
        let debug_path = format!("BigInt constant {digits}");
        let printed = JsItem {
            name: name.clone(),
            kind: ItemKind::Const,
            decl: jsast::const_(name.as_str(), Expr::BigInt(digits.to_owned())),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: debug_path.clone(),
            order: None,
            loc: None,
            names: Vec::new(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: None,
            jir_decline: None,
            jir_item: None,
        };
        let (js, mappings) = printed.to_js_mapped(crate::opts::get().comments);
        Some(LinkItem {
            name,
            kind: ItemKind::Const,
            js,
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path,
            crate_name: "generated".to_owned(),
            order: None,
            map: item_map(mappings),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: Default::default(),
        })
    }
}

/// Spells every BigInt constant of `decl` as its binding's name, and adds the digits of each one
/// it names to `used`. A constant [`BigIntNames::name`] refuses keeps its inline spelling. Verbatim
/// JavaScript (`Expr::Raw`, `Stmt::Raw`) is opaque and left alone.
pub fn hoist(decl: &mut Stmt, names: &BigIntNames, used: &mut BTreeSet<String>) {
    Hoist { names, used }.stmt(decl);
}

struct Hoist<'a> {
    names: &'a BigIntNames,
    used: &'a mut BTreeSet<String>,
}

impl Hoist<'_> {
    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::ExprStmt(expr) | Stmt::Throw(expr) | Stmt::Const(_, expr) => self.expr(expr),
            Stmt::Let(_, init) | Stmt::Return(init) => {
                if let Some(init) = init {
                    self.expr(init);
                }
            }
            Stmt::Lets(declarations) => {
                for (_, init) in declarations {
                    if let Some(init) = init {
                        self.expr(init);
                    }
                }
            }
            Stmt::If(test, then, els) => {
                self.expr(test);
                self.stmts(then);
                if let Some(els) = els {
                    self.stmts(els);
                }
            }
            Stmt::Switch(discriminant, cases) => {
                self.expr(discriminant);
                for case in cases {
                    for test in &mut case.tests {
                        self.expr(test);
                    }
                    self.stmts(&mut case.body);
                }
            }
            Stmt::Labeled(_, inner) => self.stmt(inner),
            Stmt::LoopForever(_, body) | Stmt::Block(body) => self.stmts(body),
            Stmt::While(test, body) => {
                self.expr(test);
                self.stmts(body);
            }
            Stmt::TryCatch { body, handler, .. } => {
                self.stmts(body);
                self.stmts(handler);
            }
            Stmt::FunctionDecl { body, .. } => self.stmts(body),
            Stmt::Continue(_)
            | Stmt::Break(_)
            | Stmt::Import { .. }
            | Stmt::Comment(_)
            | Stmt::Raw(_)
            | Stmt::Loc(_) => {}
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::BigInt(digits) => {
                if let Some(name) = self.names.name(digits) {
                    self.used.insert(std::mem::take(digits));
                    *expr = Expr::Ident(name);
                }
            }
            Expr::Member(object, _) | Expr::Unary(_, object) => self.expr(object),
            Expr::Index(left, right)
            | Expr::Binary(_, left, right)
            | Expr::Assign(left, right)
            | Expr::CompoundAssign(_, left, right) => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Call(callee, arguments) | Expr::New(callee, arguments) => {
                self.expr(callee);
                for argument in arguments {
                    self.expr(argument);
                }
            }
            Expr::Cond(test, consequent, alternate) => {
                self.expr(test);
                self.expr(consequent);
                self.expr(alternate);
            }
            Expr::Object(entries) => {
                for (_, value) in entries {
                    self.expr(value);
                }
            }
            Expr::Array(items) | Expr::Seq(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            Expr::Arrow(_, body) => match body {
                ArrowBody::Expr(body) => self.expr(body),
                ArrowBody::Block(body) => self.stmts(body),
            },
            Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
            | Expr::Ident(_)
            | Expr::Raw(_) => {}
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
        assert_eq!(
            printed,
            "function f(a){a=a>>($BI_33&$BI_63);return[b=>$BI_m1,$BI_33,BigInt(\"007\"),\
             (BigInt(\"5\"))]}"
        );
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
