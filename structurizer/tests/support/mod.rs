//! Building graphs and rendering the regions they structure into.

#![allow(dead_code)]

use structurizer::{
    structure, Block, BlockId, Cfg, Options, Region, Scrut, Term, Test,
};

pub type S = &'static str;
pub type R = Region<S, S>;

pub fn b(i: u32) -> BlockId {
    BlockId(i)
}

pub fn goto(t: u32) -> Term<S> {
    Term::Goto(b(t))
}

pub fn cond(on: S, then_: u32, else_: u32) -> Term<S> {
    Term::Cond { on, then_: b(then_), else_: b(else_) }
}

pub fn switch(on: S, cases: &[(u128, u32)], default: u32) -> Term<S> {
    Term::Switch {
        on,
        cases: cases.iter().map(|&(v, t)| (v, b(t))).collect(),
        default: b(default),
    }
}

pub fn ret() -> Term<S> {
    Term::Return
}

pub fn unreachable() -> Term<S> {
    Term::Unreachable
}

pub fn throw() -> Term<S> {
    Term::Throw
}

/// A graph whose blocks are named after their index.
pub fn cfg(blocks: Vec<Term<S>>) -> Cfg<S, S> {
    let names = [
        "b0", "b1", "b2", "b3", "b4", "b5", "b6", "b7", "b8", "b9", "b10", "b11", "b12", "b13",
        "b14", "b15", "b16", "b17", "b18", "b19", "b20", "b21", "b22", "b23", "b24", "b25", "b26",
        "b27", "b28", "b29", "b30", "b31",
    ];
    Cfg {
        entry: b(0),
        blocks: blocks
            .into_iter()
            .enumerate()
            .map(|(i, term)| Block { stmts: names[i], term, unwind: None })
            .collect(),
    }
}

/// A graph whose blocks carry the given statements.
pub fn cfg_with(blocks: Vec<(S, Term<S>)>) -> Cfg<S, S> {
    Cfg {
        entry: b(0),
        blocks: blocks
            .into_iter()
            .map(|(stmts, term)| Block { stmts, term, unwind: None })
            .collect(),
    }
}

/// Structures a graph and renders the result.
pub fn run(blocks: Vec<Term<S>>) -> String {
    run_opts(cfg(blocks), &Options::default())
}

pub fn run_cfg(c: Cfg<S, S>) -> String {
    run_opts(c, &Options::default())
}

/// Points expect-test at this workspace.
///
/// It otherwise walks up to the outermost enclosing `Cargo.toml`, which is the
/// one of the repository this spike lives in, and then fails to find the test
/// file it wants to rewrite. Without this, `UPDATE_EXPECT=1` cannot rebaseline.
pub fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("CARGO_WORKSPACE_DIR").is_none() {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            std::env::set_var("CARGO_WORKSPACE_DIR", root);
        }
    });
}

pub fn run_opts(c: Cfg<S, S>, opts: &Options) -> String {
    init();
    match structure(c, opts) {
        Ok(out) => {
            let mut s = String::new();
            if out.stats.irreducible {
                s.push_str("// irreducible\n");
            }
            if out.stats.dispatch_fallbacks > 0 {
                s.push_str(&format!("// dispatch x{}\n", out.stats.dispatch_fallbacks));
            }
            let used = labels(&out.region);
            if !used.is_empty() {
                let names: Vec<String> = used.iter().map(|l| format!("L{l}")).collect();
                s.push_str(&format!("// labels: {}\n", names.join(" ")));
            }
            s.push_str(&render(&out.region));
            s
        }
        Err(e) => format!("error: {e}\n"),
    }
}

/// A section of a matrix test.
pub fn section(name: &str, body: String) -> String {
    format!("== {name} ==\n{body}")
}

/// Counts of the pieces a region is made of, for outputs too wide to read.
#[derive(Default)]
pub struct Summary {
    pub blocks: u32,
    pub tests: Vec<String>,
    pub switches: u32,
    pub switch_cases: Vec<usize>,
}

pub fn summarize(r: &R) -> String {
    let mut s = Summary::default();
    walk(r, &mut s);
    format!(
        "blocks: {}\ntests: {}\nswitches: {} with {:?} cases\n",
        s.blocks,
        s.tests.join(" "),
        s.switches,
        s.switch_cases,
    )
}

fn walk(r: &R, s: &mut Summary) {
    match r {
        Region::Seq(v) => v.iter().for_each(|r| walk(r, s)),
        Region::Block(_) => s.blocks += 1,
        Region::Labeled { body, .. } | Region::Loop { body, .. } => walk(body, s),
        Region::If { test: t, then_, else_, .. } => {
            s.tests.push(
                match t {
                    Test::IsTrue => "IsTrue",
                    Test::Eq(_) => "Eq",
                    Test::Lt(_) => "Lt",
                    Test::Le(_) => "Le",
                    Test::InRange { .. } => "InRange",
                }
                .to_string(),
            );
            walk(then_, s);
            walk(else_, s);
        }
        Region::Switch { arms, default, .. } => {
            s.switches += 1;
            s.switch_cases
                .push(arms.iter().map(|(v, _)| v.len()).sum::<usize>() + 1);
            arms.iter().for_each(|(_, r)| walk(r, s));
            walk(default, s);
        }
        Region::Dispatch { cases, .. } => cases.iter().for_each(|(_, r)| walk(r, s)),
        _ => {}
    }
}

/// The labels the output actually carries, in tree order.
pub fn labels(r: &R) -> Vec<u32> {
    let mut out = Vec::new();
    collect_labels(r, &mut out);
    out
}

fn collect_labels(r: &R, out: &mut Vec<u32>) {
    match r {
        Region::Seq(v) => v.iter().for_each(|r| collect_labels(r, out)),
        Region::Labeled { label, body } => {
            out.push(label.0);
            collect_labels(body, out);
        }
        Region::Loop { label, body } => {
            out.extend(label.map(|l| l.0));
            collect_labels(body, out);
        }
        Region::If { then_, else_, .. } => {
            collect_labels(then_, out);
            collect_labels(else_, out);
        }
        Region::Switch { arms, default, label, .. } => {
            out.extend(label.map(|l| l.0));
            arms.iter().for_each(|(_, r)| collect_labels(r, out));
            collect_labels(default, out);
        }
        Region::Dispatch { label, cases, .. } => {
            out.extend(label.map(|l| l.0));
            cases.iter().for_each(|(_, r)| collect_labels(r, out));
        }
        _ => {}
    }
}

pub fn render(r: &R) -> String {
    let mut out = String::new();
    write_region(&mut out, r, 0);
    out
}

fn pad(out: &mut String, d: usize) {
    for _ in 0..d {
        out.push_str("  ");
    }
}

fn is_empty(r: &R) -> bool {
    match r {
        Region::Seq(v) => v.iter().all(is_empty),
        _ => false,
    }
}

fn scrut(s: &Scrut<S>) -> String {
    match s {
        Scrut::Payload(c) => (*c).to_string(),
        Scrut::Tmp(t) => format!("$t{}", t.0),
    }
}

fn test(s: &Scrut<S>, t: &Test) -> String {
    let x = scrut(s);
    match t {
        Test::IsTrue => x,
        Test::Eq(n) => format!("{x} == {n}"),
        Test::Lt(n) => format!("{n} < {x}"),
        Test::Le(n) => format!("{n} <= {x}"),
        Test::InRange { lo, len } => format!("{x} in [{lo}..{})", lo + len),
    }
}

fn write_region(out: &mut String, r: &R, d: usize) {
    match r {
        Region::Seq(v) => {
            for x in v {
                write_region(out, x, d);
            }
        }
        Region::Block(s) => {
            pad(out, d);
            out.push_str(s);
            out.push_str(";\n");
        }
        Region::Labeled { label, body } => {
            pad(out, d);
            out.push_str(&format!("L{}: {{\n", label.0));
            write_region(out, body, d + 1);
            pad(out, d);
            out.push_str("}\n");
        }
        Region::Loop { label, body } => {
            pad(out, d);
            match label {
                Some(l) => out.push_str(&format!("L{}: loop {{\n", l.0)),
                None => out.push_str("loop {\n"),
            }
            write_region(out, body, d + 1);
            pad(out, d);
            out.push_str("}\n");
        }
        Region::LetTmp { tmp, value } => {
            pad(out, d);
            out.push_str(&format!("let $t{} = {};\n", tmp.0, value));
        }
        Region::If { scrut: s, test: t, then_, else_ } => {
            pad(out, d);
            out.push_str(&format!("if ({}) {{\n", test(s, t)));
            write_region(out, then_, d + 1);
            pad(out, d);
            if is_empty(else_) {
                out.push_str("}\n");
            } else {
                out.push_str("} else {\n");
                write_region(out, else_, d + 1);
                pad(out, d);
                out.push_str("}\n");
            }
        }
        Region::Switch { scrut: s, arms, default, label } => {
            pad(out, d);
            if let Some(l) = label {
                out.push_str(&format!("L{}: ", l.0));
            }
            out.push_str(&format!("switch ({}) {{\n", scrut(s)));
            for (values, body) in arms {
                pad(out, d + 1);
                let vs: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                out.push_str(&format!("case {}:\n", vs.join(", ")));
                write_region(out, body, d + 2);
            }
            pad(out, d + 1);
            out.push_str("default:\n");
            write_region(out, default, d + 2);
            pad(out, d);
            out.push_str("}\n");
        }
        Region::Dispatch { sel, label, cases, trailing_break } => {
            pad(out, d);
            if let Some(l) = label {
                out.push_str(&format!("L{}: ", l.0));
            }
            out.push_str("loop {\n");
            pad(out, d + 1);
            out.push_str(&format!("switch ($t{}) {{\n", sel.0));
            for (c, body) in cases {
                pad(out, d + 2);
                out.push_str(&format!("case {c}:\n"));
                write_region(out, body, d + 3);
            }
            pad(out, d + 1);
            out.push_str("}\n");
            if *trailing_break {
                pad(out, d + 1);
                out.push_str("break;\n");
            }
            pad(out, d);
            out.push_str("}\n");
        }
        Region::SetSel(t, c) => {
            pad(out, d);
            out.push_str(&format!("$t{} = {};\n", t.0, c));
        }
        Region::Break(l) => {
            pad(out, d);
            match l {
                Some(l) => out.push_str(&format!("break L{};\n", l.0)),
                None => out.push_str("break;\n"),
            }
        }
        Region::Continue(l) => {
            pad(out, d);
            match l {
                Some(l) => out.push_str(&format!("continue L{};\n", l.0)),
                None => out.push_str("continue;\n"),
            }
        }
        Region::Return => {
            pad(out, d);
            out.push_str("return;\n");
        }
        Region::Unreachable => {
            pad(out, d);
            out.push_str("unreachable;\n");
        }
        Region::Throw => {
            pad(out, d);
            out.push_str("throw;\n");
        }
    }
}
