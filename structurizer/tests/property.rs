//! Random graphs, run twice: once as a graph, once as the region tree it
//! structures into. Both must visit the same blocks in the same order.
//!
//! This is the safety net for the whole crate: a structurizer that reorders,
//! drops or duplicates a block, or that resolves a label wrongly, shows up
//! here as a diverging trace.

use std::collections::HashMap;

use structurizer::{
    structure, Block, BlockId, Cfg, Label, Options, Payload, Region, Scrut, Term, Test,
};

/// The statements of a block, which are just its number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stmts(u32);

impl Payload for Stmts {
    fn cost(&self) -> i32 {
        1
    }
}

/// A condition payload names the block whose terminator branches on it.
type Cond = u32;

/// How far either run is allowed to go.
const MAX_TRACE: usize = 60;
const MAX_STEPS: u32 = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Returned,
    Threw,
    Unreachable,
    OutOfSteps,
}

/// A linear congruential generator, so the graphs are the same on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A graph, kept around after structuring so the interpreters can agree on
/// what a branch means.
struct Spec {
    terms: Vec<Term<Cond>>,
}

impl Spec {
    fn generate(rng: &mut Lcg) -> Spec {
        let n = 3 + rng.below(18) as u32;
        let mut terms = Vec::new();
        for i in 0..n {
            // Two thirds of the edges lead forward, which keeps the graphs
            // from being nothing but tangles.
            let target = |rng: &mut Lcg| -> BlockId {
                if rng.below(3) > 0 && i + 1 < n {
                    BlockId(i + 1 + rng.below((n - i - 1) as u64) as u32)
                } else {
                    BlockId(rng.below(n as u64) as u32)
                }
            };
            let term = match rng.below(12) {
                0..=4 => Term::Goto(target(rng)),
                5..=8 => Term::Cond { on: i, then_: target(rng), else_: target(rng) },
                9 | 10 => {
                    let k = 1 + rng.below(4) as usize;
                    let cases = (0..k)
                        .map(|j| (rng.below(6) as u128 + j as u128, target(rng)))
                        .collect();
                    Term::Switch { on: i, cases, default: target(rng) }
                }
                _ => Term::Return,
            };
            terms.push(term);
        }
        // Somewhere to end up.
        terms.push(Term::Return);
        Spec { terms }
    }

    fn cfg(&self) -> Cfg<Stmts, Cond> {
        Cfg {
            entry: BlockId(0),
            blocks: self
                .terms
                .iter()
                .enumerate()
                .map(|(i, t)| Block {
                    stmts: Stmts(i as u32),
                    term: clone_term(t),
                    unwind: None,
                })
                .collect(),
        }
    }
}

fn clone_term(t: &Term<Cond>) -> Term<Cond> {
    match t {
        Term::Goto(b) => Term::Goto(*b),
        Term::Cond { on, then_, else_ } => Term::Cond { on: *on, then_: *then_, else_: *else_ },
        Term::Switch { on, cases, default } => {
            Term::Switch { on: *on, cases: cases.clone(), default: *default }
        }
        Term::Return => Term::Return,
        Term::Unreachable => Term::Unreachable,
        Term::Throw => Term::Throw,
    }
}

/// Hands out the value of each branch from a fixed script, so both runs of a
/// graph take the same decisions.
struct Oracle<'a> {
    spec: &'a Spec,
    script: &'a [u64],
    pos: usize,
}

impl<'a> Oracle<'a> {
    fn new(spec: &'a Spec, script: &'a [u64]) -> Oracle<'a> {
        Oracle { spec, script, pos: 0 }
    }

    /// The value the terminator of `block` branches on this time round.
    fn decide(&mut self, block: u32) -> u128 {
        let d = self.script[self.pos % self.script.len()];
        self.pos += 1;
        match &self.spec.terms[block as usize] {
            Term::Cond { .. } => (d % 2) as u128,
            Term::Switch { cases, .. } => {
                let k = (d % (cases.len() as u64 + 1)) as usize;
                match cases.get(k) {
                    Some(&(v, _)) => v,
                    // A value no case lists, so the default is taken.
                    None => cases.iter().map(|(v, _)| *v).max().unwrap_or(0) + 1,
                }
            }
            _ => 0,
        }
    }
}

/// Runs the graph itself.
fn walk_cfg(spec: &Spec, script: &[u64]) -> (Vec<u32>, Outcome) {
    let mut oracle = Oracle::new(spec, script);
    let mut trace = Vec::new();
    let mut pc = 0u32;
    loop {
        if trace.len() >= MAX_TRACE {
            return (trace, Outcome::OutOfSteps);
        }
        trace.push(pc);
        pc = match &spec.terms[pc as usize] {
            Term::Goto(b) => b.0,
            Term::Cond { then_, else_, .. } => {
                if oracle.decide(pc) != 0 {
                    then_.0
                } else {
                    else_.0
                }
            }
            Term::Switch { cases, default, .. } => {
                let v = oracle.decide(pc);
                cases
                    .iter()
                    .find(|(cv, _)| *cv == v)
                    .map(|(_, t)| t.0)
                    .unwrap_or(default.0)
            }
            Term::Return => return (trace, Outcome::Returned),
            Term::Throw => return (trace, Outcome::Threw),
            Term::Unreachable => return (trace, Outcome::Unreachable),
        };
    }
}

/// How a region gave control back, following JavaScript rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Fall,
    Break(Option<Label>),
    Continue(Option<Label>),
    Done(Outcome),
}

struct Interp<'a> {
    oracle: Oracle<'a>,
    trace: Vec<u32>,
    tmps: HashMap<u32, u128>,
    steps: u32,
}

impl<'a> Interp<'a> {
    fn eval(&mut self, s: &Scrut<Cond>) -> u128 {
        match s {
            Scrut::Payload(b) => self.oracle.decide(*b),
            Scrut::Tmp(t) => self.tmps.get(&t.0).copied().unwrap_or(0),
        }
    }

    fn holds(t: &Test, v: u128) -> bool {
        match *t {
            Test::IsTrue => v != 0,
            Test::Eq(n) => v == n,
            Test::Lt(n) => n < v,
            Test::Le(n) => n <= v,
            Test::InRange { lo, len } => v >= lo && v - lo < len,
        }
    }

    /// Runs the arms of a switch from `start` on, falling through into each
    /// other until one of them leaves.
    fn arms(&mut self, arms: &[(Vec<u128>, Region<Stmts, Cond>)], start: usize) -> Flow {
        for (_, body) in &arms[start.min(arms.len())..] {
            match self.exec(body) {
                Flow::Fall => continue,
                f => return f,
            }
        }
        Flow::Fall
    }

    fn exec(&mut self, r: &Region<Stmts, Cond>) -> Flow {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Flow::Done(Outcome::OutOfSteps);
        }
        match r {
            Region::Seq(v) => {
                for x in v {
                    match self.exec(x) {
                        Flow::Fall => continue,
                        f => return f,
                    }
                }
                Flow::Fall
            }
            Region::Block(s) => {
                if self.trace.len() >= MAX_TRACE {
                    return Flow::Done(Outcome::OutOfSteps);
                }
                self.trace.push(s.0);
                Flow::Fall
            }
            Region::Labeled { label, body } => match self.exec(body) {
                Flow::Break(Some(l)) if l == *label => Flow::Fall,
                f => f,
            },
            Region::Loop { label, body } => loop {
                match self.exec(body) {
                    Flow::Fall | Flow::Continue(None) => continue,
                    Flow::Continue(Some(l)) if Some(l) == *label => continue,
                    Flow::Break(None) => return Flow::Fall,
                    Flow::Break(Some(l)) if Some(l) == *label => return Flow::Fall,
                    f => return f,
                }
            },
            Region::LetTmp { tmp, value } => {
                let v = self.oracle.decide(*value);
                self.tmps.insert(tmp.0, v);
                Flow::Fall
            }
            Region::If { scrut, test, then_, else_ } => {
                let v = self.eval(scrut);
                if Interp::holds(test, v) {
                    self.exec(then_)
                } else {
                    self.exec(else_)
                }
            }
            Region::Switch { scrut, arms, default, label } => {
                let v = self.eval(scrut);
                let start = arms
                    .iter()
                    .position(|(vs, _)| vs.contains(&v))
                    .unwrap_or(arms.len());
                let f = match self.arms(arms, start) {
                    Flow::Fall => self.exec(default),
                    f => f,
                };
                match f {
                    Flow::Break(None) => Flow::Fall,
                    Flow::Break(Some(l)) if Some(l) == *label => Flow::Fall,
                    f => f,
                }
            }
            Region::Dispatch { sel, label, cases, trailing_break } => loop {
                let v = self.tmps.get(&sel.0).copied().unwrap_or(0);
                let start = cases
                    .iter()
                    .position(|(c, _)| u128::from(*c) == v)
                    .unwrap_or(cases.len());
                let mut f = Flow::Fall;
                for (_, body) in &cases[start.min(cases.len())..] {
                    match self.exec(body) {
                        Flow::Fall => continue,
                        other => {
                            f = other;
                            break;
                        }
                    }
                }
                // Leaving the switch lands after it, inside the loop.
                match f {
                    Flow::Fall | Flow::Break(None) => {
                        if *trailing_break {
                            return Flow::Fall;
                        }
                    }
                    Flow::Continue(None) => {}
                    Flow::Continue(Some(l)) if Some(l) == *label => {}
                    Flow::Break(Some(l)) if Some(l) == *label => return Flow::Fall,
                    other => return other,
                }
                if self.steps > MAX_STEPS {
                    return Flow::Done(Outcome::OutOfSteps);
                }
            },
            Region::SetSel(t, c) => {
                self.tmps.insert(t.0, u128::from(*c));
                Flow::Fall
            }
            Region::Break(l) => Flow::Break(*l),
            Region::Continue(l) => Flow::Continue(*l),
            Region::Return => Flow::Done(Outcome::Returned),
            Region::Unreachable => Flow::Done(Outcome::Unreachable),
            Region::Throw => Flow::Done(Outcome::Threw),
        }
    }
}

/// Runs the region tree.
fn walk_region(spec: &Spec, region: &Region<Stmts, Cond>, script: &[u64]) -> (Vec<u32>, Outcome) {
    let mut interp = Interp {
        oracle: Oracle::new(spec, script),
        trace: Vec::new(),
        tmps: HashMap::new(),
        steps: 0,
    };
    let outcome = match interp.exec(region) {
        Flow::Done(o) => o,
        // Falling off the end of a body is a return.
        _ => Outcome::Returned,
    };
    (interp.trace, outcome)
}

#[test]
fn a_region_visits_the_same_blocks_as_the_graph_it_came_from() {
    let option_sets = [
        ("default", Options::default()),
        ("dispatch", Options { force_dispatch: true, ..Options::default() }),
        // Forces the flat dispatch wherever a join needs a scope, and narrow
        // switches, so the decision trees have to split.
        (
            "narrow",
            Options { merge_node_max: 0, switch_max_case: 2, ..Options::default() },
        ),
    ];
    let mut rng = Lcg(0x5eed_1234_9abc_def0);
    let mut graphs = 0;
    let mut checks = 0;
    let mut irreducible = 0;
    let mut dispatches = 0;
    let mut traced = 0;

    for _ in 0..30 {
        let spec = Spec::generate(&mut rng);
        graphs += 1;
        let scripts: Vec<Vec<u64>> = (0..8)
            .map(|_| (0..7).map(|_| rng.next()).collect())
            .collect();
        for (name, opts) in &option_sets {
            let out = match structure(spec.cfg(), opts) {
                Ok(out) => out,
                Err(e) => panic!("graph {graphs} under {name}: {e}"),
            };
            irreducible += u32::from(out.stats.irreducible);
            dispatches += out.stats.dispatch_fallbacks;
            for script in &scripts {
                let want = walk_cfg(&spec, script);
                let got = walk_region(&spec, &out.region, script);
                assert_eq!(
                    want, got,
                    "graph {graphs} under {name} with script {script:?}\n\
                     blocks: {:?}",
                    spec.terms.len()
                );
                checks += 1;
                traced += want.0.len();
            }
        }
    }

    assert_eq!(graphs, 30);
    assert_eq!(checks, 30 * 3 * 8);
    // The generator has to keep producing graphs worth checking.
    assert!(irreducible > 0, "no irreducible graph was generated");
    assert!(dispatches >= 60, "the dispatch fallback was barely exercised");
    assert!(traced > 5000, "the traces were too short to mean much: {traced}");
    println!(
        "{graphs} graphs, {checks} runs, {traced} block visits, \
         {irreducible} irreducible, {dispatches} dispatch regions"
    );
}
