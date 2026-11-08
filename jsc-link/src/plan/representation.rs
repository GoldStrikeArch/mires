//! The representation plan: solve the value graph with the min cut, choose carriers, and commit
//! them transactionally (Phase 4, D1 + D3 + D4 + D9 + D12).
//!
//! This is lane phase **B1**, run over phase A's [`Func`]s before the flat direct-call ABI (B2):
//!
//! 1. [`crate::analysis::valueflow::collect`] discovers the whole-program value graph and
//!    [`crate::analysis::legality::terms`] its legality terms; [`Problem::build`] flattens both.
//! 2. The families solve in order (D3): **F1** Baseline vs Native over every node with measured
//!    per-item byte costs and cheapest-native adapter arcs, then **F2** (vec — degenerate while
//!    every committable vec slot admits `NativeVec` only) and **F3** HostString vs StringBuilder
//!    over the F1-native string nodes. Costs are [`crate::plan::cost`]'s: the real pipeline's
//!    measured bytes, never a heuristic score. The solve→measure loop runs at most three rounds; a
//!    region whose measured Δ is not `< 0` is demoted and the families re-solve without it.
//! 3. The commit (D4) is transactional per region — a region is the must-equal-connected value
//!    component, which is a union of recursive call SCCs, so declining it atomically declines every
//!    SCC in it. Every affected `Func` is rewritten on a clone: carriers set, `Convert`s inserted
//!    at cut edges (lowered by the registry-owned adapters in `lower/js/adapters.rs`), then
//!    verified with [`Func::verify_carriers`] under exactly the committed [`CarrierPolicy`]. A
//!    verify failure rolls the whole region back (`version.carrier-commit-rollback` recorded,
//!    originals kept) and the commit rebuilds without it, so no neighbor keeps an adapter against a
//!    label that no longer exists. A producer-tagged standard-operation item whose slot committed
//!    is not rewritten as a `Func` at all: its whole body is replaced by the registry's native
//!    lowering ([`crate::lower::semantic_ops::rewrite`]) when phase C prints it.
//! 4. The sidecar's `repr` block (D12) is filled in `report` and `on` alike; under `report` the
//!    commit is computed on clones and thrown away, so `off` == `report` bytes holds while the
//!    block still says what `on` commits.
//!
//! A committed item is rewritten in place when every reachable caller agrees on its native ABI.
//! When a baseline reference must coexist with prepared native callers, D4 retains the original
//! body and emits at most one private `$n1` version. The commit closes that version set over native
//! direct calls and direct drop glue, retargets only the prepared callers, and excludes the source
//! items from the flat-ABI pass. Version naming is collision-safe and the version, retargets, and
//! carrier rewrites are measured and installed in the same transaction. An unlowered or indirect
//! reference still reaches only the baseline body.
//!
//! The `format_args!` trio (`FormatArgsNew`, `FormatArgumentNewDisplay`, `FormatArgsToString`)
//! never commits a *body*: [`solver_blocked`] keeps their own slot nodes baseline, because an
//! unproven caller may still pass baseline values into them. What commits instead is the **call
//! site** (W4g, `lower/semantic_ops/fmt_kernel.rs`): a proven site's edges into the machinery
//! are exempted from the legality terms, the machinery an all-proven rewrite stops referencing
//! is priced as droppable dependency nodes, and the commit marks the site's constructor-call
//! results with native carriers — the in-band signal `lower/js/call.rs` inlines the site by.
//! Every machinery item stays printed at baseline for whatever unproven callers remain; reach
//! drops it when none do.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::analysis::legality::{self, EqualReason, Term};
use crate::analysis::valueflow::{self, CandidateKind, NodeKey, Site, Step, UnionFind, ValueGraph};
use crate::item::{prologue, write_program, JsName, LinkItem};
use crate::jir::decline::{CostDecline, Decline, VersionDecline};
use crate::jir::interns::Interns;
use crate::jir::repr::{Carrier, CarrierPolicy, Func, Op, OpKind, Proj, Root, ValueDef, ValueId};
use crate::jir::semantic::{
    BodySemantic, CallSemantic, Callee, DropGlue, ItemId, StandardOperation,
};
use crate::jir::types::{RepresentationCandidate, TypeId, TypeTable};
use crate::jsast;
use crate::link::lane::{ReprBlock, ReprDecision, ReprExport};
use crate::lower::js::adapters;
use crate::lower::semantic_ops::{
    self, LowerCx, ReferenceRequirement, SignatureSlot, SlotRequirement,
};
use crate::lower::LowerInput;
use crate::plan::cost::{self, INF};
use crate::plan::graph::{Cap, Problem, ProblemKey};
use crate::plan::{mincut, versions};

/// What one lane pass hands B1: the option set and the program-wide indexes the cost model's
/// phase-C probe lowerings read.
pub(crate) struct ReprCx<'a> {
    pub opts: &'a crate::opts::Opts,
    pub items: &'a crate::lower::js::host::ItemIndex,
    pub drop_glue: &'a crate::lower::js::host::DropGlueIndex,
    pub vtables: &'a crate::lower::js::unsize::VtableIndex,
}

/// Why one function's commit refused: the problem nodes whose regions must roll back beside the
/// item's own (a cut standard call names its callee's too), and the reason.
type CommitDecline = (BTreeSet<usize>, String);

/// A committed standard-operation item's replacement body, installed by phase C.
#[derive(Clone, Debug)]
pub(crate) struct NativeBody {
    pub parameters: Vec<String>,
    pub statements: Vec<jsast::Stmt>,
}

/// Everything B1 decided. `bodies` and `touched` are empty unless the plan applied.
#[derive(Debug, Default)]
pub(crate) struct ReprOutcome {
    pub block: ReprBlock,
    /// Every item whose printed body this plan changes: rewritten `Func`s and replaced
    /// standard-operation bodies. A phase-C decline of one of these retries the run with both
    /// plans emptied (D1).
    pub touched: BTreeSet<JsName>,
    /// The standard-operation items phase C prints from the registry instead of their `Func`.
    pub bodies: BTreeMap<JsName, NativeBody>,
    /// Private native version name -> retained baseline source item.
    pub clones: BTreeMap<JsName, JsName>,
    /// Direct-call targets retargeted only in prepared JIR callers.
    pub call_retargets: BTreeMap<JsName, crate::lower::CallRetargets>,
}

impl ReprOutcome {
    /// Keeps the proposed and validated accounting while applying no representation rewrite.
    /// This is `js-opt=report`: it describes what `on` would emit and prints baseline bytes.
    pub fn unapplied(&self) -> ReprOutcome {
        let mut block = self.block.clone();
        block.applied = 0;
        block.emits = false;
        ReprOutcome {
            block,
            touched: BTreeSet::new(),
            bodies: BTreeMap::new(),
            clones: BTreeMap::new(),
            call_retargets: BTreeMap::new(),
        }
    }

    /// What a lane retry falls back to: the accounting kept, the commit disowned. Every decision
    /// the commit had kept is re-marked as rolled back, so the sidecar describes the program
    /// that was actually printed.
    pub fn emptied(&self) -> ReprOutcome {
        let mut block = self.block.clone();
        block.committed = 0;
        block.adapters = 0;
        block.versions = 0;
        block.helper_bytes = 0;
        block.applied = 0;
        block.emits = true;
        block.bytes_after = block.bytes_before;
        block.score_after = block.score_before;
        for decision in &mut block.decisions {
            if decision.decline.is_none() && decision.chosen != "baseline" {
                decision.chosen = "baseline".to_owned();
                decision.decline = Some(
                    "version.carrier-commit-rollback a phase-C decline emptied the plan".to_owned(),
                );
            }
        }
        // The printed program is baseline's: the published export contract must say so.
        for export in &mut block.exports {
            export.chosen = "baseline".to_owned();
        }
        ReprOutcome {
            block,
            touched: BTreeSet::new(),
            bodies: BTreeMap::new(),
            clones: BTreeMap::new(),
            call_retargets: BTreeMap::new(),
        }
    }
}

/// The one entry point: solve, account, and — when `apply` — commit into `funcs`.
///
/// With `apply` false (`js-opt=report`) the whole plan still runs, the commit included, on
/// clones that are then dropped: the sidecar must say what `on` would do, and a committed count
/// that skipped the verify would not.
pub(crate) fn plan_and_commit(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    live: &BTreeSet<JsName>,
    cx: &ReprCx<'_>,
    apply: bool,
) -> ReprOutcome {
    let interns_map: BTreeMap<JsName, &Interns> = table
        .iter()
        .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
        .collect();
    let mut graph = valueflow::collect(table, funcs, &interns_map, live);
    free_version_pins(&mut graph);
    if let Ok(filter) = std::env::var("RCGJS_REPR_DEBUG_FUNC") {
        for (name, func) in funcs.iter() {
            if name.as_str().contains(&filter) {
                if let Ok(func) = func {
                    eprintln!("repr-func: {} = {func:#?}", name.as_str());
                    if let Some(semantic) = table.get(name).and_then(|item| item.jir.body.as_ref())
                    {
                        eprintln!(
                            "repr-func-semantic: {} {:?}",
                            name.as_str(),
                            semantic.semantic
                        );
                    }
                }
            }
        }
    }
    if let Ok(filter) = std::env::var("RCGJS_REPR_DEBUG_GRAPH") {
        for line in graph.dump().lines() {
            if line.contains(&filter) {
                eprintln!("repr-graph: {line}");
            }
        }
        for (key, facts) in &graph.nodes {
            if !key.to_string().contains(&filter) {
                continue;
            }
            let item = match key {
                NodeKey::Value { item, .. } => Some(item),
                _ => None,
            };
            if let Some(descriptor) = item
                .and_then(|item| interns_map.get(item))
                .and_then(|interns| facts.ty.and_then(|ty| interns.types.get(ty)))
            {
                eprintln!(
                    "repr-graph-type: {key} {:?} {:?} {:?}",
                    descriptor.meta, descriptor.layout, descriptor.kind
                );
            }
        }
    }
    let lowered: BTreeMap<ItemId, JsName> = funcs
        .iter()
        .filter_map(|(name, func)| Some((func.as_ref().ok()?.item, name.clone())))
        .collect();
    // The call-site format kernel (W4g): the proven `format_args!` sites, recognized before the
    // legality terms so a proven site's edges into the machinery items never reach the solver.
    let fmt = fmt_analyses(table, funcs, &interns_map, &lowered, &graph);
    let mut terms = legality::terms(&graph);
    filter_format_terms(&mut terms, &fmt, &lowered);
    let mut problem = Problem::build(&graph, &terms);

    let (outcome, committed_funcs) = {
        let planner = Planner {
            table,
            funcs,
            live,
            cx,
            interns: &interns_map,
            graph: &graph,
            lowered,
            fmt: &fmt,
            member_bytes: cost::helper_bytes(),
            reached: reached_members(table),
        };
        planner.run(&mut problem, &terms, apply)
    };
    if apply {
        for (name, func) in committed_funcs {
            funcs.insert(name, Ok(func));
        }
        outcome
    } else {
        outcome.unapplied()
    }
}

/// A public/unmodeled reference pins the original ABI, not a private linker-owned version. Keep
/// the boundary set as clone evidence, but remove exactly that unary from the solve; every other
/// legality reason remains and can still decline the region.
fn free_version_pins(graph: &mut ValueGraph) {
    for key in graph.pinned_boundaries.clone() {
        if let Some(facts) = graph.nodes.get_mut(&key) {
            facts.reasons.remove(&legality::InfReason::UnknownCall);
        }
    }
}

/// Every `__rt` member the baseline program already spells, so reaching it costs the plan
/// nothing (the same subtraction `scripts/minimal-shim.mjs` makes).
fn reached_members(table: &BTreeMap<JsName, LinkItem>) -> BTreeSet<String> {
    let mut direct = BTreeSet::new();
    for item in table.values() {
        direct.extend(rt_members(&item.js));
    }
    cost::helper_closure(&direct)
}

/// Every `__rt.<ident>` a piece of JavaScript text mentions.
fn rt_members(text: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("__rt.") {
        rest = &rest[at + 5..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(rest.len());
        if end > 0 {
            members.push(rest[..end].to_owned());
        }
    }
    members
}

/// Makes the non-emitting table placeholder for a private native version. Phase C replaces its
/// declaration before it can become reachable; retaining the source's JIR handle lets the clone
/// lower from the same verified body and intern tables.
pub(crate) fn clone_link_item(mut source: LinkItem, name: &JsName) -> LinkItem {
    source.name = name.clone();
    source.linkage = crate::item::Linkage::internal();
    source.order = None;
    source.dts = None;
    source.debug_path = format!("{} [native version]", source.debug_path);
    source
}

/// The compact declaration used only by D9's hypothetical program table.
fn install_compact(item: &mut LinkItem, lowered: crate::lower::LoweredItem) {
    let mut decl = lowered.decl;
    crate::minify::shorten_locals_for_measurement(&mut decl);
    item.js = jsast::program_to_string_styled_for_target(
        std::slice::from_ref(&decl),
        jsast::Style::Compact,
        crate::opts::get().ecma_target,
    );
    item.refs = lowered.refs;
}

/// A registry replacement as the ordinary lowering record D9 installs in its cloned table.
fn native_lowered(name: &JsName, body: &NativeBody) -> crate::lower::LoweredItem {
    let decl = jsast::function(
        name.as_str().to_owned(),
        body.parameters.clone(),
        body.statements.clone(),
    );
    let mut refs = BTreeSet::new();
    jsast::visit_idents(&decl, &mut |ident| {
        if ident != name.as_str() {
            refs.insert(JsName::new(ident.to_owned()));
        }
    });
    crate::lower::LoweredItem {
        decl,
        refs,
        names: Vec::new(),
        loc: None,
        dts: None,
        zombies: Vec::new(),
        js_alloc_origins: Vec::new(),
    }
}

/// Final reachability for a hypothetical program, without diagnostics for missing extra roots.
pub(crate) fn measurement_reach(
    table: &BTreeMap<JsName, LinkItem>,
    extra_roots: &[String],
) -> Vec<JsName> {
    let mut queue: VecDeque<JsName> = table
        .iter()
        .filter(|(name, item)| {
            item.linkage.root || extra_roots.iter().any(|root| root == name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();
    let mut seen = Vec::new();
    let mut in_set: BTreeSet<JsName> = queue.iter().cloned().collect();
    while let Some(name) = queue.pop_front() {
        seen.push(name.clone());
        let Some(item) = table.get(&name) else {
            continue;
        };
        for referenced in &item.refs {
            if table.contains_key(referenced) && in_set.insert(referenced.clone()) {
                queue.push_back(referenced.clone());
            }
        }
    }
    seen
}

/// One committable standard-operation item: the problem node of its one slot and the measured
/// bytes of every body it could take.
struct OpEntry {
    name: JsName,
    node: usize,
    /// The item's baseline body, measured through the real pipeline.
    baseline: u64,
    /// Measured native body bytes and the rewrite, per carrier the slots admit, in slot order.
    native: Vec<(Carrier, u64, semantic_ops::Rewrite)>,
    /// The registry members the native bodies may reach, for the helper facility nodes.
    helpers: BTreeSet<&'static str>,
}

impl OpEntry {
    fn native_at(&self, carrier: Carrier) -> Option<&(Carrier, u64, semantic_ops::Rewrite)> {
        self.native.iter().find(|(at, _, _)| *at == carrier)
    }

    fn cheapest_native(&self) -> u64 {
        self.native
            .iter()
            .map(|(_, bytes, _)| *bytes)
            .min()
            .unwrap_or(INF)
    }
}

/// One priced pairwise term between two problem nodes: `ab` paid with `a` native beside `b`
/// baseline (the mincut convention).
#[derive(Clone, Copy, Debug)]
struct Pair {
    a: usize,
    b: usize,
    ab: u64,
    ba: u64,
    /// If both endpoints survive F1 as native strings, F3 must give them one concrete carrier.
    /// Reference calls still permit a private callee version when only the caller is native.
    native_equal: bool,
}

enum Direction {
    NativeToBaseline,
    BaselineToNative,
}

/// The droppable-dependency nodes and their implication arcs.
#[derive(Default)]
struct Dependencies {
    nodes: BTreeMap<JsName, DepNode>,
    pairs: Vec<Pair>,
}

struct DepNode {
    node: usize,
    bytes: u64,
    /// The transitive candidate-operation slot nodes whose commits let this item drop: what the
    /// per-region credit reads.
    users: Vec<usize>,
}

/// What the commit produced.
#[derive(Default)]
struct Commit {
    funcs: BTreeMap<JsName, Func>,
    bodies: BTreeMap<JsName, NativeBody>,
    /// Private native version name -> retained baseline source item.
    clones: BTreeMap<JsName, JsName>,
    /// Per emitted caller, the direct items whose private native versions it targets.
    call_retargets: BTreeMap<JsName, crate::lower::CallRetargets>,
    adapters: u32,
    /// Every shim member the committed bodies and adapters reach.
    members: BTreeSet<String>,
    /// Exact compiler-link artifact costs before and after this label vector.
    measured: Option<(ProgramCost, ProgramCost)>,
}

/// One retained standard-operation source that a native reference caller reaches through a
/// private version. All callers agree on `carrier`, so D4's one-version budget is sufficient.
#[derive(Clone, Debug)]
struct NativeVersionNeed {
    carrier: Carrier,
    callers: BTreeSet<JsName>,
    nodes: BTreeSet<usize>,
}

/// The exact compiler-link artifact cost of one fully lowered label vector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProgramCost {
    /// Final reachable JavaScript after link deduplication and optional item renaming, excluding
    /// the runtime-helper closure recorded separately below.
    bytes: u64,
    /// Exact emitted bytes of the runtime-helper closure reached by that JavaScript.
    helper_bytes: u64,
    /// Deterministic gzip-9 bytes of the complete compiler-link artifact.
    gzip: u64,
    /// Deterministic Brotli-11 bytes of the complete compiler-link artifact.
    brotli: u64,
    /// Final statically present JavaScript allocation sites.
    allocations: u64,
    /// Final allocation sites lexically or transitively carried by a loop.
    loop_allocations: u64,
    /// The configured allocation weight over the final optimized JIR-R functions.
    alloc_term: u64,
}

impl ProgramCost {
    fn emitted_bytes(self) -> u64 {
        self.bytes.saturating_add(self.helper_bytes)
    }

    fn total(self) -> u64 {
        self.emitted_bytes().saturating_add(self.alloc_term)
    }
}

/// The release profile's carrier-selection economics.
///
/// The exact v1/v2 runtime authority currently admits no size-for-speed budget. These policies
/// therefore differ only inside a zero-growth envelope: size accepts a strict raw, gzip, or
/// Brotli Pareto win, balanced can value any allocation win, and speed requires the stronger
/// loop-carried allocation evidence. Static hotness is not permission to grow the artifact; a
/// future runtime receipt must establish that permission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CarrierEconomics {
    Size,
    Balanced,
    Speed,
}

impl CarrierEconomics {
    fn from_opts(opts: &crate::opts::Opts) -> CarrierEconomics {
        match opts.profile {
            Some(crate::opts::Profile::ReleaseSize) => Self::Size,
            Some(crate::opts::Profile::ReleaseSpeed) => Self::Speed,
            Some(crate::opts::Profile::Dev | crate::opts::Profile::Report)
            | Some(crate::opts::Profile::ReleaseBalanced)
            | None => Self::Balanced,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Size => "release-size",
            Self::Balanced => "release-balanced",
            Self::Speed => "release-speed",
        }
    }

    fn accepts(self, before: ProgramCost, after: ProgramCost) -> bool {
        if after.emitted_bytes() > before.emitted_bytes()
            || after.gzip > before.gzip
            || after.brotli > before.brotli
        {
            return false;
        }

        match self {
            Self::Size => {
                after.emitted_bytes() < before.emitted_bytes()
                    || after.gzip < before.gzip
                    || after.brotli < before.brotli
            }
            Self::Balanced => {
                after.allocations <= before.allocations
                    && after.loop_allocations <= before.loop_allocations
                    && after.total() < before.total()
            }
            Self::Speed => {
                after.allocations <= before.allocations
                    && after.loop_allocations <= before.loop_allocations
                    && (after.emitted_bytes() < before.emitted_bytes()
                        || after.loop_allocations < before.loop_allocations)
            }
        }
    }

    fn benefit(self, before: ProgramCost, after: ProgramCost) -> u64 {
        match self {
            Self::Size => before
                .emitted_bytes()
                .saturating_sub(after.emitted_bytes())
                .saturating_add(before.gzip.saturating_sub(after.gzip))
                .saturating_add(before.brotli.saturating_sub(after.brotli)),
            Self::Balanced => before.total().saturating_sub(after.total()),
            Self::Speed => before
                .loop_allocations
                .saturating_sub(after.loop_allocations)
                .saturating_mul(1_000_000)
                .saturating_add(before.emitted_bytes().saturating_sub(after.emitted_bytes())),
        }
    }
}

struct Planner<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    live: &'a BTreeSet<JsName>,
    cx: &'a ReprCx<'a>,
    interns: &'a BTreeMap<JsName, &'a Interns>,
    graph: &'a ValueGraph,
    /// Every lowered item by its [`ItemId`].
    lowered: BTreeMap<ItemId, JsName>,
    /// The proven format sites per consumer item (W4g).
    fmt: &'a BTreeMap<JsName, semantic_ops::fmt_kernel::Analysis>,
    member_bytes: BTreeMap<String, u64>,
    reached: BTreeSet<String>,
}

impl Planner<'_> {
    fn run(
        &self,
        problem: &mut Problem,
        terms: &[Term],
        apply: bool,
    ) -> (ReprOutcome, BTreeMap<JsName, Func>) {
        let ops = self.op_entries(problem);
        let regions = regions(problem, terms);
        anchor_guard(problem, &regions, &ops);
        let helper_nodes = self.helper_nodes(problem, &ops);
        let deps = self.dependency_nodes(problem, &ops);
        let mut pairs = self.pairs(problem, terms);
        pairs.extend(deps.pairs.iter().copied());
        pairs.extend(self.summarized_call_pairs(problem));
        let versionable: BTreeSet<JsName> = ops.keys().cloned().collect();
        pairs.extend(self.standard_call_pairs(problem, &versionable));
        pairs.extend(self.fmt_site_pairs(problem));
        let unaries = self.f1_unaries(problem, &ops, &helper_nodes, &deps);

        // The solve→measure loop (D9, ≤3 rounds). Every term is already a measured byte figure,
        // so re-solving cannot move a number; the loop's work is demoting regions that fail the
        // active release profile's exact compiler-link economics and solving again without them.
        if std::env::var("RCGJS_REPR_DEBUG_SOLVE").is_ok() {
            for (index, node) in problem.nodes.iter().enumerate() {
                let (b, n) = (unaries.0[index], unaries.1[index]);
                if b > 0 || (n > 0 && n < INF) {
                    let n = if n >= INF {
                        "INF".to_owned()
                    } else {
                        n.to_string()
                    };
                    eprintln!("repr-solve: n{index} {} b={b} n={n}", node.key);
                }
            }
            for pair in &pairs {
                let cap = |cap: u64| {
                    if cap >= INF {
                        "INF".to_owned()
                    } else {
                        cap.to_string()
                    }
                };
                eprintln!(
                    "repr-solve: arc n{} <{}/{}> n{}",
                    pair.a,
                    cap(pair.ab),
                    cap(pair.ba),
                    pair.b
                );
            }
            for (index, node) in problem.nodes.iter().enumerate() {
                if unaries.1[index] >= INF {
                    eprintln!("repr-solve: pinned n{index} {}", node.key);
                }
            }
        }
        let measured_baseline = self.program_cost(&Commit::default());
        let exact_available = measured_baseline.is_some();
        let baseline = measured_baseline.unwrap_or_else(|| self.fallback_program_cost());
        let economics = CarrierEconomics::from_opts(self.cx.opts);
        let count = problem.nodes.len();
        let baseline_labels = vec![None; count];
        let mut measured: BTreeMap<Vec<Option<Carrier>>, Option<ProgramCost>> =
            BTreeMap::from([(baseline_labels, Some(baseline))]);
        let measure_vector = |labels: &[Option<Carrier>]| {
            if !exact_available {
                return None;
            }
            match self.build_commit(problem, labels, &ops, false) {
                Ok(commit) => self.program_cost(&commit),
                Err((_, reason)) => {
                    if std::env::var("RCGJS_REPR_DEBUG_COST").is_ok() {
                        eprintln!("repr-cost: commit declined: {reason}");
                    }
                    None
                }
            }
        };
        let mut forced: BTreeSet<usize> = BTreeSet::new();
        let mut demoted: BTreeMap<usize, String> = BTreeMap::new();
        let mut rollbacks: BTreeMap<usize, String> = BTreeMap::new();
        let mut no_carrier: BTreeSet<usize> = BTreeSet::new();
        let mut proposed: BTreeSet<usize> = BTreeSet::new();
        let mut carriers = vec![None; count];
        let mut families = 0;
        let mut economic_rounds = 0;
        let fault = std::env::var("RCGJS_REPR_FAULT").is_ok_and(|value| value == "verify");
        let mut accepted = None;

        while economic_rounds < 3 {
            let (solved, f1, ran, infeasible) =
                self.solve_families(problem, &ops, &pairs, &unaries, &forced);
            carriers = solved;
            families = ran;
            if !infeasible.is_empty() {
                let mut added = false;
                for region in &regions {
                    if region.iter().any(|node| infeasible.contains(node)) {
                        for node in region {
                            added |= forced.insert(*node);
                            no_carrier.insert(*node);
                        }
                    }
                }
                if added {
                    continue;
                }
            }
            if std::env::var("RCGJS_REPR_DEBUG_SOLVE").is_ok() {
                for (index, native) in f1.iter().enumerate() {
                    if *native {
                        eprintln!(
                            "repr-solve: f1-native n{index} {}",
                            problem.nodes[index].key
                        );
                    }
                }
            }
            proposed.extend(
                carriers
                    .iter()
                    .enumerate()
                    .filter_map(|(node, carrier)| carrier.is_some().then_some(node)),
            );

            let native_side: Vec<bool> = (0..count)
                .map(|index| {
                    carriers[index].is_some()
                        || (f1[index] && problem.nodes[index].family.is_none())
                })
                .collect();
            let components: Vec<BTreeSet<usize>> = native_components(&pairs, &native_side)
                .into_iter()
                .filter(|component| component.iter().any(|node| carriers[*node].is_some()))
                .collect();
            if components.is_empty() {
                let commit = Commit {
                    measured: Some((baseline, baseline)),
                    ..Commit::default()
                };
                accepted = Some(commit);
                break;
            }

            let mut demote_now: BTreeMap<usize, String> = BTreeMap::new();
            let mut component_savings = Vec::new();
            for component in &components {
                let mut labels = vec![None; count];
                for node in component {
                    labels[*node] = carriers[*node];
                }
                let cost = if let Some(cost) = measured.get(&labels) {
                    *cost
                } else {
                    let cost = measure_vector(&labels);
                    measured.insert(labels, cost);
                    cost
                };
                let profitable = cost.is_some_and(|cost| economics.accepts(baseline, cost));
                if std::env::var("RCGJS_REPR_DEBUG_COST").is_ok() {
                    eprintln!(
                        "repr-cost: component {:?}: before={baseline:?} after={cost:?}",
                        component
                    );
                }
                if profitable {
                    component_savings.push((
                        component,
                        economics.benefit(baseline, cost.expect("checked")),
                    ));
                    continue;
                }
                let decline = exact_cost_decline(economics, baseline, cost);
                for node in component {
                    if matches!(problem.nodes[*node].key, ProblemKey::Value(_)) {
                        demote_now.entry(*node).or_insert_with(|| decline.clone());
                    }
                }
            }
            if !demote_now.is_empty() {
                economic_rounds += 1;
                forced.extend(demote_now.keys().copied());
                demoted.extend(demote_now);
                continue;
            }

            let full_cost = if let Some(cost) = measured.get(&carriers) {
                *cost
            } else {
                let cost = measure_vector(&carriers);
                measured.insert(carriers.clone(), cost);
                cost
            };
            let profitable = full_cost.is_some_and(|cost| economics.accepts(baseline, cost));
            if std::env::var("RCGJS_REPR_DEBUG_COST").is_ok() {
                eprintln!("repr-cost: full: before={baseline:?} after={full_cost:?}");
            }
            if !profitable {
                economic_rounds += 1;
                let Some((weakest, _)) = component_savings
                    .into_iter()
                    .min_by_key(|(_, saving)| *saving)
                else {
                    break;
                };
                let decline = exact_cost_decline(economics, baseline, full_cost);
                for node in weakest {
                    if matches!(problem.nodes[*node].key, ProblemKey::Value(_)) {
                        forced.insert(*node);
                        demoted.entry(*node).or_insert_with(|| decline.clone());
                    }
                }
                continue;
            }

            match self.build_commit(problem, &carriers, &ops, fault) {
                Ok(mut commit) => {
                    commit.measured = Some((baseline, full_cost.expect("checked")));
                    accepted = Some(commit);
                    break;
                }
                Err((failed, reason)) => {
                    let decline = format!(
                        "{} {reason}",
                        Decline::Version(VersionDecline::CarrierCommitRollback {
                            reason: String::new(),
                        })
                        .code()
                    );
                    let mut hit = false;
                    for region in &regions {
                        if region.iter().any(|node| failed.contains(node)) {
                            for node in region {
                                if carriers[*node].is_some() {
                                    hit |= forced.insert(*node);
                                    rollbacks.entry(*node).or_insert_with(|| decline.clone());
                                }
                            }
                        }
                    }
                    if !hit {
                        break;
                    }
                    // The next iteration re-solves the economics without the rolled-back region.
                }
            }
        }

        let commit = accepted.unwrap_or_else(|| {
            carriers.iter_mut().for_each(|carrier| *carrier = None);
            Commit {
                measured: Some((baseline, baseline)),
                ..Commit::default()
            }
        });

        let block = self.account(
            problem,
            &regions,
            &carriers,
            &pairs,
            &unaries,
            &demoted,
            &rollbacks,
            &no_carrier,
            &proposed,
            &commit,
            families,
        );
        let outcome = ReprOutcome {
            block,
            touched: commit
                .funcs
                .keys()
                .chain(commit.bodies.keys())
                .cloned()
                .collect(),
            bodies: commit.bodies,
            clones: commit.clones,
            call_retargets: commit.call_retargets,
        };
        let funcs = if apply { commit.funcs } else { BTreeMap::new() };
        (outcome, funcs)
    }

    // -- costs ---------------------------------------------------------------------------------

    /// The committable standard-operation items and their measured bodies.
    fn op_entries(&self, problem: &Problem) -> BTreeMap<JsName, OpEntry> {
        let mut entries = BTreeMap::new();
        for (name, func) in self.funcs {
            let Ok(func) = func else { continue };
            let Some(item) = self.table.get(name) else {
                continue;
            };
            let (Some(body), Some(interns)) = (item.jir.body.as_ref(), item.jir.interns()) else {
                continue;
            };
            let probe = LowerCx {
                semantic: body.semantic,
                sig: &body.signature,
                table: &interns.types,
                interns,
                element_glue: None,
            };
            let Ok((op, slots)) = semantic_ops::support_of(&probe) else {
                continue;
            };
            // The exact monomorphic child drop glue, where the operation needs one.
            let required_glue = match op.deps {
                semantic_ops::Deps::ElementDropGlue => {
                    semantic_ops::drop::required_element_drop(&probe)
                }
                semantic_ops::Deps::BoxPointeeDropGlue => {
                    semantic_ops::drop::required_box_pointee_drop(&probe)
                }
                semantic_ops::Deps::None => Ok(None),
            };
            let glue_name: Option<JsName> = required_glue
                .ok()
                .flatten()
                .and_then(|element| self.cx.drop_glue.get(&element))
                .map(|glue| JsName::new(glue.clone()));
            let lcx = LowerCx {
                element_glue: glue_name.as_ref(),
                ..probe
            };
            if solver_blocked(op.id) {
                continue;
            }
            let Some(node) = slots
                .first()
                .and_then(|slot| self.slot_node(problem, name, func, slot))
            else {
                // No candidate slot node: nothing the solver could label.
                continue;
            };
            let Some(input) = self.lower_input(name, item) else {
                continue;
            };
            let Some(baseline) = cost::measure_item(&input, func) else {
                continue;
            };
            let slot_carriers = slots.first().map(|slot| slot.carriers).unwrap_or_default();
            let mut native = Vec::new();
            let mut helpers = BTreeSet::new();
            for carrier in slot_carriers {
                let selected: Vec<Carrier> = slots.iter().map(|_| *carrier).collect();
                if let Ok(rewrite) = semantic_ops::rewrite(&lcx, &selected) {
                    let decl = jsast::function(
                        name.as_str().to_owned(),
                        rewrite.parameters.clone(),
                        rewrite.statements.clone(),
                    );
                    helpers.extend(rewrite.runtime_helpers.iter().copied());
                    native.push((*carrier, cost::measure_decl(&decl), rewrite));
                }
            }
            if native.is_empty() {
                continue;
            }
            entries.insert(
                name.clone(),
                OpEntry {
                    name: name.clone(),
                    node,
                    baseline,
                    native,
                    helpers,
                },
            );
        }
        entries
    }

    /// The implication arcs a SUMMARIZED body's own calls owe (D4's coherence): the value-flow
    /// analysis summarizes a standard-operation or candidate-glue body instead of collecting it,
    /// so a call that body makes — the String drop glue dropping its inner `Vec<u8>` through the
    /// vec glue — is invisible to the edge model. While the summarized body stays baseline that
    /// hidden call passes baseline values, so its callee may go native only when the summarized
    /// caller does (whose native body makes no such call): `callee-node native beside
    /// summarized-caller-slot baseline` is INF. A summarized caller with no slot node pins the
    /// callee outright.
    fn summarized_call_pairs(&self, problem: &mut Problem) -> Vec<Pair> {
        let mut pairs = Vec::new();
        for (name, func) in self.funcs {
            let Ok(func) = func else { continue };
            if !self.is_candidate_glue_or_standard(name) {
                continue;
            }
            let caller_slots: Vec<usize> = problem
                .resolution
                .iter()
                .filter(|(key, _)| matches!(key, NodeKey::Value { item, .. } if item == name))
                .map(|(_, index)| *index)
                .collect();
            let mut callees: BTreeSet<&JsName> = BTreeSet::new();
            for block in &func.blocks {
                for op in &block.ops {
                    let (OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        ..
                    }
                    | OpKind::Drop {
                        glue: DropGlue::Direct { item, .. },
                        ..
                    }) = &op.kind
                    else {
                        continue;
                    };
                    if let Some(callee) = self.lowered.get(item) {
                        if callee != name {
                            callees.insert(callee);
                        }
                    }
                }
            }
            for callee in callees {
                let callee_nodes: Vec<usize> = problem
                    .resolution
                    .iter()
                    .filter(|(key, _)| matches!(key, NodeKey::Value { item, .. } if item == callee))
                    .map(|(_, index)| *index)
                    .collect();
                for node in callee_nodes {
                    if caller_slots.is_empty() {
                        problem.nodes[node]
                            .inf
                            .insert(crate::analysis::legality::InfReason::UnknownCall);
                        continue;
                    }
                    for slot in &caller_slots {
                        if node != *slot {
                            pairs.push(Pair {
                                a: node,
                                b: *slot,
                                ab: INF,
                                ba: 0,
                                native_equal: false,
                            });
                        }
                    }
                }
            }
        }
        pairs
    }

    /// The coherence arcs of the D2 cut: one asymmetric implication per slot requirement of
    /// every registry-covered `CallSemantic::Standard` call in an ordinary (collected) body.
    /// Direction follows [`Self::summarized_call_pairs`] (`ab: INF, ba: 0`, `a` the callee): the
    /// callee item's slot goes native only when every cut caller's does — its one body must not
    /// receive a baseline value from any caller — while a callee that CANNOT go native (an
    /// untagged alloc-internal body poisoned by its own raw views) permits a native reference
    /// caller only when the registry produced an executable private version. Otherwise that
    /// reference boundary is must-equal and the baseline callee pins the caller. A remaining
    /// mixed labeling is spelled by a boundary conversion at value slots and by one private
    /// native callee version at reference slots. Native reference callers of the same slot are
    /// F3-equal, so one clone is enough.
    fn standard_call_pairs(&self, problem: &Problem, versionable: &BTreeSet<JsName>) -> Vec<Pair> {
        let mut pairs = Vec::new();
        let mut reference_callers: BTreeMap<(JsName, usize), Vec<usize>> = BTreeMap::new();
        for (name, func) in self.funcs {
            let Ok(func) = func else { continue };
            if self.is_candidate_glue_or_standard(name) {
                continue;
            }
            for block in &func.blocks {
                for op in &block.ops {
                    let OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        args,
                        semantic: CallSemantic::Standard(operation),
                        host: None,
                        ..
                    } = &op.kind
                    else {
                        continue;
                    };
                    let Some(entry) = semantic_ops::registry(*operation) else {
                        continue;
                    };
                    let Some(callee_name) = self.lowered.get(item) else {
                        continue;
                    };
                    let Some(Ok(callee_func)) = self.funcs.get(callee_name) else {
                        continue;
                    };
                    if !call_aligns(callee_func, args.len()) {
                        continue;
                    }
                    let cuttable = *operation == StandardOperation::StringAsStr
                        || versionable.contains(callee_name);
                    for (slot_index, requirement) in entry.slots.iter().enumerate() {
                        let caller = site_slot_key(name, args, op.result, requirement)
                            .and_then(|key| problem.resolution.get(&key))
                            .copied();
                        let callee_node =
                            self.slot_node(problem, callee_name, callee_func, requirement);
                        match (caller, callee_node) {
                            (Some(caller), Some(callee)) if caller != callee => {
                                if requirement.reference != ReferenceRequirement::Value
                                    && *operation != StandardOperation::StringAsStr
                                    && cuttable
                                {
                                    reference_callers
                                        .entry((callee_name.clone(), slot_index))
                                        .or_default()
                                        .push(caller);
                                }
                                pairs.push(Pair {
                                    a: callee,
                                    b: caller,
                                    ab: INF,
                                    ba: if requirement.reference != ReferenceRequirement::Value
                                        && !cuttable
                                    {
                                        INF
                                    } else {
                                        0
                                    },
                                    native_equal: requirement.reference
                                        != ReferenceRequirement::Value,
                                });
                            }
                            // A reference slot whose callee has no node cannot be
                            // label-checked at commit: the caller never goes native against it.
                            (Some(caller), None)
                                if requirement.reference != ReferenceRequirement::Value =>
                            {
                                pairs.push(Pair {
                                    a: caller,
                                    b: problem.baseline,
                                    ab: INF,
                                    ba: 0,
                                    native_equal: false,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        for callers in reference_callers.values_mut() {
            callers.sort_unstable();
            callers.dedup();
            for window in callers.windows(2) {
                if window[0] != window[1] {
                    // F1-free: either caller may stay baseline. F3-equal: if both go native,
                    // their shared private callee version needs one concrete carrier.
                    pairs.push(Pair {
                        a: window[0],
                        b: window[1],
                        ab: 0,
                        ba: 0,
                        native_equal: true,
                    });
                }
            }
        }
        pairs
    }

    /// The must-equal coupling of one proven site's participants — the destination (or the
    /// `write_fmt` receiver's pointee) and every `String` display source: the site inlines all
    /// of them at native carriers or none at all, so a mixed labeling is never solved into
    /// (and the commit rolls the region back if one appears anyway).
    fn fmt_site_pairs(&self, problem: &Problem) -> Vec<Pair> {
        let mut pairs = Vec::new();
        for (name, analysis) in self.fmt {
            for site in &analysis.sites {
                let Some(keys) = fmt_site_keys(name, site) else {
                    continue;
                };
                let nodes: Vec<usize> = keys
                    .iter()
                    .filter_map(|key| problem.resolution.get(key).copied())
                    .collect();
                for window in nodes.windows(2) {
                    if window[0] != window[1] {
                        pairs.push(Pair {
                            a: window[0],
                            b: window[1],
                            ab: INF,
                            ba: INF,
                            native_equal: true,
                        });
                    }
                }
            }
        }
        pairs
    }

    /// The problem node a slot requirement's value collapsed into.
    fn slot_node(
        &self,
        problem: &Problem,
        name: &JsName,
        func: &Func,
        slot: &SlotRequirement,
    ) -> Option<usize> {
        let (site, path) = match slot.slot {
            SignatureSlot::Result => (Site::Ret, Vec::new()),
            SignatureSlot::Argument(index) => {
                let param = *func.params.get(usize::from(index))?;
                let path = if slot.reference == ReferenceRequirement::Value {
                    Vec::new()
                } else {
                    vec![Step::Deref]
                };
                (Site::Value(param), path)
            }
        };
        problem
            .resolution
            .get(&NodeKey::Value {
                item: name.clone(),
                site,
                path,
            })
            .copied()
    }

    fn lower_input<'i>(&'i self, name: &'i JsName, item: &'i LinkItem) -> Option<LowerInput<'i>> {
        self.lower_input_with(
            name,
            item,
            self.cx.opts,
            &[],
            crate::lower::EMPTY_CALL_RETARGETS,
        )
    }

    fn lower_input_with<'i>(
        &'i self,
        name: &'i JsName,
        item: &'i LinkItem,
        opts: &'i crate::opts::Opts,
        spills: &'i [JsName],
        call_retargets: &'i crate::lower::CallRetargets,
    ) -> Option<LowerInput<'i>> {
        let interns = item.jir.interns()?;
        Some(LowerInput {
            name,
            summary: item.jir.summary.as_ref()?,
            body: item.jir.body.as_ref()?,
            origins: item.jir.origins.as_ref(),
            interns,
            table: &interns.types,
            opts,
            items: self.cx.items,
            drop_glue: self.cx.drop_glue,
            vtables: self.cx.vtables,
            spills,
            abi: None,
            physical: None,
            call_retargets,
        })
    }

    /// One `Helper` facility node per registry member a native body may reach.
    fn helper_nodes(
        &self,
        problem: &mut Problem,
        ops: &BTreeMap<JsName, OpEntry>,
    ) -> BTreeMap<String, usize> {
        let mut users: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for entry in ops.values() {
            let direct: BTreeSet<String> = entry
                .helpers
                .iter()
                .map(|member| (*member).to_owned())
                .collect();
            for member in cost::helper_closure(&direct) {
                users.entry(member).or_default().push(entry.node);
            }
        }
        let mut nodes = BTreeMap::new();
        for (member, mut using) in users {
            using.sort_unstable();
            using.dedup();
            nodes.insert(member.clone(), problem.add_helper(&member, &using));
        }
        nodes
    }

    /// The items that are reachable only through the baseline bodies of the candidate
    /// operations: the program drops them when their users commit, which is the native side's
    /// real saving (v1's "complete reachable item graph" cost, as cut nodes).
    ///
    /// The walk starts from the program's root shape — every live item nothing live references,
    /// plus everything exported — and traverses baseline `refs`, except that a candidate
    /// operation contributes its NATIVE body's item references instead. What the walk cannot
    /// reach is only alive through baseline op bodies; each such item becomes a
    /// [`ProblemKey::Dependency`] node whose baseline cost is its measured bytes and whose
    /// implication arcs say "this drops only when every referencing user goes native".
    fn dependency_nodes(
        &self,
        problem: &mut Problem,
        ops: &BTreeMap<JsName, OpEntry>,
    ) -> Dependencies {
        // The live reference graph.
        let live_refs = |name: &JsName| -> Vec<JsName> {
            self.table
                .get(name)
                .map(|item| {
                    item.refs
                        .iter()
                        .filter(|reference| self.live.contains(*reference))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut referenced: BTreeSet<JsName> = BTreeSet::new();
        for name in self.live {
            if !self.table.contains_key(name) {
                continue;
            }
            referenced.extend(live_refs(name));
        }
        // Roots: live items nothing live references, plus the exported.
        let roots: Vec<&JsName> = self
            .live
            .iter()
            .filter(|name| {
                self.table
                    .get(*name)
                    .is_some_and(|item| !referenced.contains(*name) || item.linkage.exported)
            })
            .collect();
        // The native refs an op item keeps: its rewrites' item references, all carriers.
        let native_refs: BTreeMap<&JsName, BTreeSet<JsName>> = ops
            .iter()
            .map(|(name, entry)| {
                let mut refs = BTreeSet::new();
                for (_, _, rewrite) in &entry.native {
                    refs.extend(rewrite.item_references.iter().cloned());
                }
                (name, refs)
            })
            .collect();
        // A proven format consumer keeps everything but the machinery its committed sites stop
        // calling (W4g): its contribution to the walk is its refs minus the elidable callees.
        let mut fmt_kept: BTreeMap<&JsName, BTreeSet<JsName>> = BTreeMap::new();
        for (name, analysis) in self.fmt {
            let Some(item) = self.table.get(name) else {
                continue;
            };
            let elided: BTreeSet<JsName> = analysis
                .elidable
                .iter()
                .filter_map(|id| self.lowered.get(id).cloned())
                .collect();
            if elided.is_empty() {
                continue;
            }
            let kept = item
                .refs
                .iter()
                .filter(|reference| !elided.contains(*reference))
                .cloned()
                .collect();
            fmt_kept.insert(name, kept);
        }
        let mut visited: BTreeSet<&JsName> = BTreeSet::new();
        let mut queue: Vec<&JsName> = roots;
        while let Some(name) = queue.pop() {
            if !visited.insert(name) {
                continue;
            }
            match native_refs.get(name).or_else(|| fmt_kept.get(name)) {
                Some(kept) => {
                    for reference in kept {
                        if let Some((found, _)) = self.table.get_key_value(reference) {
                            if self.live.contains(found) {
                                queue.push(found);
                            }
                        }
                    }
                }
                None => {
                    for reference in &self.table[name].refs {
                        if let Some((found, _)) = self.table.get_key_value(reference) {
                            if self.live.contains(found) && !visited.contains(found) {
                                queue.push(found);
                            }
                        }
                    }
                }
            }
        }
        let mut droppable: BTreeSet<&JsName> = self
            .live
            .iter()
            .filter(|name| self.table.contains_key(*name) && !visited.contains(*name))
            .collect();
        // In-edges within the program: a droppable item must be referenced only by candidate
        // operations and other droppable items, and by at least one of them — anything else is a
        // root shape the walk under-approximated, pruned with everything it reaches.
        // ponytail: the pruning rescans the whole table per droppable candidate (O(S²·items) set
        // lookups); a reverse-edge index pays off when a program's droppable set grows past a
        // few hundred items.
        loop {
            let mut pruned = false;
            let snapshot: Vec<&JsName> = droppable.iter().copied().collect();
            for name in snapshot {
                let mut supported = false;
                let mut foreign = false;
                for (from, item) in self.table {
                    if !self.live.contains(from) || from == name || !item.refs.contains(name) {
                        continue;
                    }
                    if ops.contains_key(from)
                        || droppable.contains(from)
                        || fmt_kept.get(from).is_some_and(|kept| !kept.contains(name))
                    {
                        supported = true;
                    } else {
                        foreign = true;
                    }
                }
                if foreign || !supported {
                    droppable.remove(name);
                    pruned = true;
                }
            }
            if !pruned {
                // A proven site's destination node stands in for the consumer as a dependency
                // user: the machinery drops only when the sites that stop calling it commit.
                let mut fmt_users: BTreeMap<&JsName, Vec<usize>> = BTreeMap::new();
                for (name, analysis) in self.fmt {
                    let mut nodes = Vec::new();
                    for site in &analysis.sites {
                        if let Some(node) = fmt_site_keys(name, site)
                            .and_then(|keys| problem.resolution.get(&keys[0]).copied())
                        {
                            nodes.push(node);
                        }
                    }
                    if !nodes.is_empty() {
                        fmt_users.insert(name, nodes);
                    }
                }
                return self.build_dependencies(problem, ops, &droppable, &fmt_users);
            }
        }
    }

    fn build_dependencies(
        &self,
        problem: &mut Problem,
        ops: &BTreeMap<JsName, OpEntry>,
        droppable: &BTreeSet<&JsName>,
        fmt_users: &BTreeMap<&JsName, Vec<usize>>,
    ) -> Dependencies {
        let mut deps = Dependencies::default();
        let mut node_of: BTreeMap<&JsName, usize> = BTreeMap::new();
        for name in droppable {
            let Some(bytes) = self.item_bytes(name) else {
                continue;
            };
            let node = problem.nodes.len();
            problem.nodes.push(crate::plan::graph::ProblemNode {
                key: ProblemKey::Dependency((*name).clone()),
                family: None,
                mutable: false,
                fixed: None,
                inf: BTreeSet::new(),
                members: 0,
            });
            node_of.insert(name, node);
            deps.nodes.insert(
                (*name).clone(),
                DepNode {
                    node,
                    bytes,
                    users: Vec::new(),
                },
            );
        }
        // Implication arcs: the dependency drops only when every referencing user goes native.
        // Users are the op slot nodes (directly) and other dependency nodes (chained); the
        // per-region credit reads the transitive op users.
        let mut direct: BTreeMap<&JsName, (Vec<usize>, Vec<&JsName>)> = BTreeMap::new();
        for name in node_of.keys().copied().collect::<Vec<_>>() {
            let mut op_users = Vec::new();
            let mut dep_users = Vec::new();
            for (from, item) in self.table {
                if !self.live.contains(from) || from == name || !item.refs.contains(name) {
                    continue;
                }
                if let Some(entry) = ops.get(from) {
                    op_users.push(entry.node);
                } else if let Some(destinations) = fmt_users.get(from) {
                    op_users.extend(destinations.iter().copied());
                } else if node_of.contains_key(from) {
                    dep_users.push(
                        node_of
                            .get_key_value(from)
                            .map(|(key, _)| *key)
                            .expect("just checked"),
                    );
                }
            }
            direct.insert(name, (op_users, dep_users));
        }
        for (name, (op_users, dep_users)) in &direct {
            let node = node_of[*name];
            for user in op_users {
                deps.pairs.push(Pair {
                    a: node,
                    b: *user,
                    ab: INF,
                    ba: 0,
                    native_equal: false,
                });
            }
            for dep_user in dep_users {
                deps.pairs.push(Pair {
                    a: node,
                    b: node_of[*dep_user],
                    ab: INF,
                    ba: 0,
                    native_equal: false,
                });
            }
        }
        // Transitive op users, for the region credit.
        for name in node_of.keys().copied().collect::<Vec<_>>() {
            let mut users: BTreeSet<usize> = BTreeSet::new();
            let mut stack = vec![name];
            let mut seen: BTreeSet<&JsName> = BTreeSet::new();
            while let Some(at) = stack.pop() {
                if !seen.insert(at) {
                    continue;
                }
                if let Some((op_users, dep_users)) = direct.get(at) {
                    users.extend(op_users.iter().copied());
                    stack.extend(dep_users.iter().copied());
                }
            }
            deps.nodes.get_mut(name).expect("a dependency node").users =
                users.into_iter().collect();
        }
        deps
    }

    /// One live item's measured compact bytes, through the same pipeline the sidecar's
    /// `bytes_min` uses. `None` for an item that cannot be measured (a synthesized body).
    fn item_bytes(&self, name: &JsName) -> Option<u64> {
        let item = self.table.get(name)?;
        let input = self.lower_input(name, item)?;
        match self.funcs.get(name) {
            Some(Ok(func)) => cost::measure_item(&input, func),
            _ => crate::lower::lower_item(&input)
                .ok()
                .map(|lowered| cost::measure_decl(&lowered.decl)),
        }
    }

    /// Lowers and measures the complete program for one committed label vector.
    ///
    /// This is the acceptance-grade D9 measurement. It includes ordinary rewritten bodies,
    /// adapters, the flat ABI, final reachability, link deduplication, profile-selected item
    /// naming, unique runtime helpers, and the allocation term. A decline anywhere in a rewritten
    /// function makes the vector unmeasurable and therefore unprofitable.
    fn program_cost(&self, commit: &Commit) -> Option<ProgramCost> {
        crate::profile::bump(crate::profile::Counter::TrialReprProgram);
        let mut opts = self.cx.opts.clone();
        opts.optimizer = crate::opts::OptimizerMode::On;
        if !matches!(
            opts.profile,
            Some(
                crate::opts::Profile::ReleaseSize
                    | crate::opts::Profile::ReleaseBalanced
                    | crate::opts::Profile::ReleaseSpeed
            )
        ) {
            // Preserve the historical compact probe for unprofiled developer/report invocations.
            opts.minify = true;
            opts.minify_items = true;
        }

        let mut table = self.table.clone();
        for (clone, source) in &commit.clones {
            let source = table.get(source)?.clone();
            table.insert(clone.clone(), clone_link_item(source, clone));
        }
        let mut funcs = self.funcs.clone();
        for (name, func) in &commit.funcs {
            funcs.insert(name.clone(), Ok(func.clone()));
        }
        let mut abi_funcs = funcs.clone();
        for clone in commit.clones.keys() {
            abi_funcs.remove(clone);
        }
        let clone_sources: BTreeSet<ItemId> = commit
            .clones
            .values()
            .filter_map(|source| self.funcs.get(source)?.as_ref().ok().map(|func| func.item))
            .collect();
        let abi = crate::plan::abi_flat::plan_excluding(&table, &abi_funcs, &opts, &clone_sources);
        abi.rewrite(&mut funcs, &table);

        let mut lowered = Vec::new();
        let mut allocation_terms: BTreeMap<JsName, (u64, u64, u64)> = BTreeMap::new();
        let mut direct_calls: BTreeMap<JsName, BTreeSet<JsName>> = BTreeMap::new();
        let mut loop_calls: BTreeMap<JsName, BTreeSet<JsName>> = BTreeMap::new();
        for (name, entry) in &funcs {
            if let Some(body) = commit.bodies.get(name) {
                let native = native_lowered(name, body);
                let sites = crate::analysis::js_alloc::JsAllocSites::of(&native.decl);
                allocation_terms.insert(
                    name.clone(),
                    (
                        sites.total(),
                        sites.in_loop(),
                        cost::Cost::new(&opts).alloc_term(&sites),
                    ),
                );
                direct_calls.insert(
                    name.clone(),
                    sites
                        .direct_calls()
                        .iter()
                        .map(|callee| JsName::new(callee.clone()))
                        .collect(),
                );
                loop_calls.insert(
                    name.clone(),
                    sites
                        .loop_calls()
                        .iter()
                        .map(|callee| JsName::new(callee.clone()))
                        .collect(),
                );
                lowered.push((name.clone(), native));
                continue;
            }
            let Ok(func) = entry else {
                if commit.funcs.contains_key(name) || abi.touches(name) {
                    return None;
                }
                continue;
            };
            let item = table.get(name)?;
            let retargets = commit
                .call_retargets
                .get(name)
                .unwrap_or(crate::lower::EMPTY_CALL_RETARGETS);
            let input = self.lower_input_with(name, item, &opts, abi.spill_names(), retargets)?;
            crate::profile::bump(crate::profile::Counter::LowerReprProgram);
            let measured = match cost::lower_item(&input, func) {
                Ok(measured) => measured,
                Err(decline) => {
                    if std::env::var("RCGJS_REPR_DEBUG_COST").is_ok() {
                        eprintln!(
                            "repr-cost: decline {} (carrier={}, abi={}): {decline}",
                            name.as_str(),
                            commit.funcs.contains_key(name),
                            abi.touches(name),
                        );
                    }
                    if commit.funcs.contains_key(name) || abi.touches(name) {
                        return None;
                    }
                    continue;
                }
            };
            let sites = crate::analysis::js_alloc::JsAllocSites::of(&measured.lowered.decl);
            allocation_terms.insert(
                name.clone(),
                (sites.total(), sites.in_loop(), measured.alloc_term),
            );
            direct_calls.insert(
                name.clone(),
                sites
                    .direct_calls()
                    .iter()
                    .map(|callee| JsName::new(callee.clone()))
                    .collect(),
            );
            loop_calls.insert(
                name.clone(),
                sites
                    .loop_calls()
                    .iter()
                    .map(|callee| JsName::new(callee.clone()))
                    .collect(),
            );
            lowered.push((name.clone(), measured.lowered));
        }

        for (name, lowered) in lowered {
            let item = table.get_mut(&name)?;
            install_compact(item, lowered);
        }
        for spill in abi.spill_items() {
            table.insert(spill.name.clone(), spill);
        }

        let mut reachable = measurement_reach(&table, &opts.roots);
        if opts.analyzes(crate::opts::OptimizerPass::LinkDedup)
            && crate::link_dedup::merge_duplicates_for_measurement(
                &mut table,
                &reachable,
                opts.source_map,
            )
        {
            reachable = measurement_reach(&table, &opts.roots);
        }

        let stubs = crate::link::stub_missing(&table, &reachable);
        let mut items = crate::link::order(&table, &reachable);
        items.extend(stubs.iter());
        let (mut program, _) = write_program(&prologue("representation-cost"), &items);
        if opts.minify_items {
            let renames = crate::minify::item_renames(&program, &items);
            program = crate::minify::apply_renames(&program, &renames);
        }
        let bytes = program.len() as u64;
        let direct_helpers = cost::runtime_members(&program);
        let helper_program = cost::helper_program(&direct_helpers);
        let helper_bytes = helper_program.len() as u64;
        program.push_str(&helper_program);

        // A callee's allocation is loop-carried when the final program calls it, transitively,
        // from a structured loop. Count each reachable allocation once and retain the declaration's
        // higher lexical-depth weight when it exceeds the transitive depth-one lower bound.
        let mut hot_items = BTreeSet::new();
        let mut hot_work: Vec<JsName> = reachable
            .iter()
            .flat_map(|caller| loop_calls.get(caller).into_iter().flatten().cloned())
            .collect();
        while let Some(name) = hot_work.pop() {
            if !reachable.contains(&name) || !hot_items.insert(name.clone()) {
                continue;
            }
            hot_work.extend(direct_calls.get(&name).into_iter().flatten().cloned());
        }
        let (allocations, loop_allocations, alloc_term) = reachable.iter().fold(
            (0u64, 0u64, 0u64),
            |(allocations, loop_allocations, alloc_term), name| {
                let (item_allocations, item_loops, item_term) =
                    allocation_terms.get(name).copied().unwrap_or_default();
                let hot = hot_items.contains(name);
                (
                    allocations.saturating_add(item_allocations),
                    loop_allocations.saturating_add(if hot {
                        item_allocations
                    } else {
                        item_loops
                    }),
                    alloc_term.saturating_add(if hot {
                        item_term.max(
                            item_allocations
                                .saturating_mul(opts.weight_alloc)
                                .saturating_mul(4),
                        )
                    } else {
                        item_term
                    }),
                )
            },
        );
        let metrics = crate::plan::physical_cost::program_metrics(
            &program,
            allocations,
            loop_allocations,
            alloc_term,
        );
        debug_assert_eq!(metrics.min, bytes.saturating_add(helper_bytes));
        Some(ProgramCost {
            bytes,
            helper_bytes,
            gzip: metrics.gzip,
            brotli: metrics.brotli,
            allocations,
            loop_allocations,
            alloc_term,
        })
    }

    /// Conservative baseline used only when a synthetic or already-declined JIR body cannot pass
    /// the measurement pipeline. Any native vector then measures as `None` and is declined; this
    /// fallback can never authorize a representation commit.
    fn fallback_program_cost(&self) -> ProgramCost {
        let items: Vec<&LinkItem> = self
            .live
            .iter()
            .filter_map(|name| self.table.get(name))
            .collect();
        let (mut program, _) = write_program(&prologue("representation-cost"), &items);
        let bytes = program.len() as u64;
        let direct_helpers = cost::runtime_members(&program);
        let helper_program = cost::helper_program(&direct_helpers);
        let helper_bytes = helper_program.len() as u64;
        program.push_str(&helper_program);
        let metrics = crate::plan::physical_cost::program_metrics(&program, 0, 0, 0);
        ProgramCost {
            bytes,
            helper_bytes,
            gzip: metrics.gzip,
            brotli: metrics.brotli,
            allocations: 0,
            loop_allocations: 0,
            alloc_term: 0,
        }
    }

    /// The F1 unary costs: measured bytes on the standard-operation slot nodes, member bytes on
    /// the helper facilities, INF wherever a native label has no lowering, zero elsewhere.
    fn f1_unaries(
        &self,
        problem: &Problem,
        ops: &BTreeMap<JsName, OpEntry>,
        helpers: &BTreeMap<String, usize>,
        deps: &Dependencies,
    ) -> (Vec<u64>, Vec<u64>) {
        let count = problem.nodes.len();
        let mut baseline_costs = vec![0u64; count];
        let mut native_costs = vec![0u64; count];
        native_costs[problem.baseline] = INF;
        for (index, node) in problem.nodes.iter().enumerate() {
            if !node.inf.is_empty() {
                native_costs[index] = INF;
            }
        }
        for entry in ops.values() {
            if native_costs[entry.node] >= INF {
                continue;
            }
            baseline_costs[entry.node] = baseline_costs[entry.node].saturating_add(entry.baseline);
            native_costs[entry.node] =
                native_costs[entry.node].saturating_add(entry.cheapest_native());
        }
        // A standard-operation or candidate-glue body is replaced whole or not at all: every
        // node it owns beyond its committed slot stays baseline, and an item the registry has no
        // entry for (the vec cluster until wave 2, the format trio, an unsupported shape) keeps
        // all of its nodes baseline.
        let committed_ops: BTreeSet<&JsName> = ops.keys().collect();
        let released: BTreeSet<usize> = ops.values().map(|entry| entry.node).collect();
        let semantic_items: BTreeSet<&JsName> = self
            .funcs
            .keys()
            .filter(|name| self.is_candidate_glue_or_standard(name))
            .collect();
        for (key, index) in &problem.resolution {
            let NodeKey::Value { item, .. } = key else {
                continue;
            };
            if !semantic_items.contains(item) {
                continue;
            }
            if committed_ops.contains(item) && released.contains(index) {
                continue;
            }
            native_costs[*index] = INF;
        }
        for (member, node) in helpers {
            if !self.reached.contains(member) {
                native_costs[*node] = native_costs[*node].saturating_add(self.member_cost(member));
            }
        }
        // A dependency's baseline cost is the bytes the program pays while any of its users keeps
        // the baseline body; going "native" is the item dropping out at cost zero.
        for dep in deps.nodes.values() {
            baseline_costs[dep.node] = dep.bytes;
            native_costs[dep.node] = 0;
        }
        (baseline_costs, native_costs)
    }

    /// Whether a body is semantic-tagged over candidate values (a standard operation, or
    /// registry-owned candidate drop glue).
    fn is_candidate_glue_or_standard(&self, name: &JsName) -> bool {
        let Some(item) = self.table.get(name) else {
            return false;
        };
        let Some(body) = item.jir.body.as_ref() else {
            return false;
        };
        match body.semantic {
            BodySemantic::Standard(_) => true,
            BodySemantic::DropGlue(ty) => item
                .jir
                .interns()
                .and_then(|interns| interns.types.get(ty))
                .is_some_and(|descriptor| match descriptor.meta.candidate {
                    RepresentationCandidate::None => false,
                    RepresentationCandidate::Box => true,
                    RepresentationCandidate::Vec | RepresentationCandidate::String => {
                        !descriptor.layout.has_own_drop
                    }
                }),
            BodySemantic::Ordinary => false,
        }
    }

    /// The legality terms re-priced with measured adapter bytes, plus the facility implication
    /// arcs the problem carries.
    fn pairs(&self, problem: &Problem, terms: &[Term]) -> Vec<Pair> {
        let mut pairs = Vec::new();
        for term in terms {
            match term {
                // A fixed carrier constrains the family sub-solve through the problem node
                // (`ProblemNode::fixed`), never native-vs-baseline: no F1 arc.
                Term::UnaryInf { .. } | Term::FixedCarrier { .. } => {}
                Term::MustEqual { a, b, .. } => {
                    let (Some(a), Some(b)) = (self.resolve(problem, a), self.resolve(problem, b))
                    else {
                        continue;
                    };
                    if a != b {
                        pairs.push(Pair {
                            a,
                            b,
                            ab: INF,
                            ba: INF,
                            native_equal: true,
                        });
                    }
                }
                Term::Adapter {
                    a,
                    b,
                    baseline_to_native_legal,
                } => {
                    let (Some(ai), Some(bi)) = (self.resolve(problem, a), self.resolve(problem, b))
                    else {
                        continue;
                    };
                    if ai == bi {
                        continue;
                    }
                    let ab = self.adapter_price(a, b, Direction::NativeToBaseline);
                    let ba = if *baseline_to_native_legal {
                        self.adapter_price(a, b, Direction::BaselineToNative)
                    } else {
                        INF
                    };
                    pairs.push(Pair {
                        a: ai,
                        b: bi,
                        ab,
                        ba,
                        native_equal: false,
                    });
                }
            }
        }
        // Facility implications (`Version`, `Helper`): INF user -> facility, free back; the
        // facility's price is its unary cost.
        for arc in &problem.arcs {
            let facility = |index: usize| {
                matches!(
                    problem.nodes[index].key,
                    ProblemKey::Version { .. } | ProblemKey::Helper(_) | ProblemKey::Dependency(_)
                )
            };
            if facility(arc.a) || facility(arc.b) {
                let cap = |cap: Cap| match cap {
                    Cap::Free | Cap::Finite => 0,
                    Cap::Inf => INF,
                };
                pairs.push(Pair {
                    a: arc.a,
                    b: arc.b,
                    ab: cap(arc.ab),
                    ba: cap(arc.ba),
                    native_equal: false,
                });
            }
        }
        pairs
    }

    fn resolve(&self, problem: &Problem, key: &NodeKey) -> Option<usize> {
        problem.resolution.get(key).copied()
    }

    /// The measured bytes of the cheapest F1 conversion an adapter edge could spell, INF where
    /// none can be spelled at all (a nested path, a group or static endpoint, a family the wave
    /// has no adapters for).
    fn adapter_price(&self, a: &NodeKey, b: &NodeKey, direction: Direction) -> u64 {
        let Some((item, ty)) = self.adapter_site(a, b) else {
            return INF;
        };
        let Some(interns) = self.interns.get(&item) else {
            return INF;
        };
        let kind = self
            .graph
            .nodes
            .get(a)
            .and_then(|facts| facts.kind)
            .or_else(|| self.graph.nodes.get(b).and_then(|facts| facts.kind));
        let native_carriers: &[Carrier] = match kind {
            Some(CandidateKind::String) => &[Carrier::HostString, Carrier::StringBuilder],
            Some(CandidateKind::Vec) => &[Carrier::NativeVec],
            Some(CandidateKind::Box) => &[Carrier::NativeBox],
            _ => return INF,
        };
        native_carriers
            .iter()
            .map(|carrier| {
                let (from, to) = match direction {
                    Direction::NativeToBaseline => (*carrier, Carrier::Baseline),
                    Direction::BaselineToNative => (Carrier::Baseline, *carrier),
                };
                self.conversion_bytes(&interns.types, interns, ty, from, to)
            })
            .min()
            .unwrap_or(INF)
    }

    /// Where a cut on this edge would be spelled, or `None` when it cannot be: the item holding
    /// the empty-path value a `Convert` can wrap, and the value's Rust type.
    fn adapter_site(&self, a: &NodeKey, b: &NodeKey) -> Option<(JsName, TypeId)> {
        let value_site = |key: &NodeKey| match key {
            NodeKey::Value { item, site, path } if path.is_empty() => match site {
                Site::Value(_) | Site::Cell(_) => Some(item.clone()),
                Site::Ret => None,
            },
            _ => None,
        };
        let empty_path = |key: &NodeKey| match key {
            NodeKey::Value { path, .. } | NodeKey::Opaque { path, .. } => path.is_empty(),
            NodeKey::Group { .. } | NodeKey::Static { .. } => false,
            NodeKey::Baseline => true,
        };
        let glue_deref = |key: &NodeKey| {
            matches!(key, NodeKey::Value { site: Site::Value(_), path, .. }
                if path.as_slice() == [Step::Deref])
        };
        // The convert wraps a whole value: the caller side of a call argument or result, or the
        // dropped value of a drop edge whose far side is the glue's dereferenced receiver.
        let site = value_site(a).or_else(|| value_site(b))?;
        let spellable = (empty_path(a) && empty_path(b))
            || (empty_path(a) && glue_deref(b))
            || (glue_deref(a) && empty_path(b));
        if !spellable {
            return None;
        }
        let ty = self
            .graph
            .nodes
            .get(a)
            .and_then(|facts| facts.ty)
            .or_else(|| self.graph.nodes.get(b).and_then(|facts| facts.ty))?;
        Some((site, ty))
    }

    /// The compact bytes of one conversion's spelling.
    ///
    /// Runtime members are program facilities, not edge costs. The exact post-commit measurement
    /// charges the union of members once, after every crossing adapter is known.
    fn conversion_bytes(
        &self,
        table: &TypeTable,
        interns: &Interns,
        ty: TypeId,
        from: Carrier,
        to: Carrier,
    ) -> u64 {
        let Ok(expr) = adapters::convert(table, interns, jsast::id("$cv"), ty, from, to) else {
            return INF;
        };
        cost::measure_expr(&expr)
    }

    fn member_cost(&self, member: &str) -> u64 {
        self.member_bytes.get(member).copied().unwrap_or(0)
    }

    // -- the solves ----------------------------------------------------------------------------

    /// F1 then F3 (F2 has no wave-1 survivors): the final carrier per problem node, and how many
    /// families ran.
    fn solve_families(
        &self,
        problem: &Problem,
        ops: &BTreeMap<JsName, OpEntry>,
        pairs: &[Pair],
        unaries: &(Vec<u64>, Vec<u64>),
        forced: &BTreeSet<usize>,
    ) -> (Vec<Option<Carrier>>, Vec<bool>, u32, BTreeSet<usize>) {
        let count = problem.nodes.len();
        let (baseline_costs, native_costs) = unaries;
        let mut cut = mincut::MinCut::new(count);
        for index in 0..count {
            let native = if forced.contains(&index) {
                INF
            } else {
                native_costs[index]
            };
            // A forced node with an INF baseline cannot exist: baseline costs are measured item
            // bytes, always finite.
            cut.unary(index, baseline_costs[index], native);
        }
        for pair in pairs {
            cut.pair(pair.a, pair.b, pair.ab, pair.ba);
        }
        cut.solve();
        let mut f1 = cut.native_side();
        let mut infeasible: BTreeSet<usize> = cut.infeasible().iter().copied().collect();
        for node in &infeasible {
            f1[*node] = false;
        }
        let mut families = 0;
        if problem.nodes.iter().any(|node| node.family.is_some()) {
            families += 1;
        }

        let mut carriers: Vec<Option<Carrier>> = vec![None; count];

        // F2 (vec): every vec slot in the registry admits `NativeVec` only — `SliceView` never
        // appears at a committable slot — so the family's NativeVec-vs-SliceView choice is
        // degenerate and every F1-native vec node takes `NativeVec`.
        let vec_survivors: Vec<usize> = (0..count)
            .filter(|index| f1[*index] && problem.nodes[*index].family == Some(CandidateKind::Vec))
            .collect();
        if !vec_survivors.is_empty() {
            families += 1;
            for node in &vec_survivors {
                carriers[*node] = Some(Carrier::NativeVec);
            }
        }

        // Box keeps its canonical pointer shape. Its native label is an ownership proof that
        // permits compiler drop glue to omit an unobservable allocator release.
        let box_survivors: Vec<usize> = (0..count)
            .filter(|index| f1[*index] && problem.nodes[*index].family == Some(CandidateKind::Box))
            .collect();
        if !box_survivors.is_empty() {
            families += 1;
            for node in &box_survivors {
                carriers[*node] = Some(Carrier::NativeBox);
            }
        }

        // F3 (string): StringBuilder (source side) vs HostString among the F1-native strings.
        let survivors: Vec<usize> = (0..count)
            .filter(|index| {
                f1[*index] && problem.nodes[*index].family == Some(CandidateKind::String)
            })
            .collect();
        if survivors.is_empty() {
            return (carriers, f1, families, infeasible);
        }
        families += 1;
        let sub: BTreeMap<usize, usize> = survivors
            .iter()
            .enumerate()
            .map(|(subindex, node)| (*node, subindex))
            .collect();
        let by_node: BTreeMap<usize, &OpEntry> =
            ops.values().map(|entry| (entry.node, entry)).collect();
        let mut cut = mincut::MinCut::new(survivors.len());
        for (subindex, node) in survivors.iter().enumerate() {
            // Host cost is paid on the sink (HostString) side, builder cost on the source side.
            let mut host = 0u64;
            let mut builder = 0u64;
            if problem.nodes[*node].mutable {
                host = INF;
            }
            if let Some(entry) = by_node.get(node) {
                let of = |carrier: Carrier| {
                    entry
                        .native_at(carrier)
                        .map(|(_, bytes, _)| *bytes)
                        .unwrap_or(INF)
                };
                host = host.saturating_add(of(Carrier::HostString)).min(INF);
                builder = builder.saturating_add(of(Carrier::StringBuilder)).min(INF);
            }
            // A host sink slot that accepts exactly one carrier fixed the node to it: the other
            // native string carrier must never be chosen (legality already pinned the mutable +
            // fixed-HostString combination, so the fixed side stays satisfiable).
            let fixed = problem.nodes[*node].fixed;
            match fixed {
                Some(Carrier::HostString) => builder = INF,
                Some(Carrier::StringBuilder) => host = INF,
                _ => {}
            }
            cut.unary(subindex, host, builder);
        }
        for pair in pairs {
            let (Some(a), Some(b)) = (sub.get(&pair.a), sub.get(&pair.b)) else {
                continue;
            };
            // Must-equal stays INF; an adapter between two native strings is the builder <->
            // host conversion, cheap in both directions.
            let (ab, ba) = if pair.native_equal || (pair.ab >= INF && pair.ba >= INF) {
                (INF, INF)
            } else {
                (
                    self.f3_conversion(Carrier::StringBuilder, Carrier::HostString),
                    self.f3_conversion(Carrier::HostString, Carrier::StringBuilder),
                )
            };
            cut.pair(*a, *b, ab, ba);
        }
        cut.solve();
        let builder_side = cut.native_side();
        let f3_infeasible: BTreeSet<usize> = cut
            .infeasible()
            .iter()
            .map(|subindex| survivors[*subindex])
            .collect();
        infeasible.extend(f3_infeasible.iter().copied());
        for (subindex, node) in survivors.iter().enumerate() {
            if f3_infeasible.contains(node) {
                f1[*node] = false;
                continue;
            }
            carriers[*node] = Some(if builder_side[subindex] {
                Carrier::StringBuilder
            } else {
                Carrier::HostString
            });
        }
        (carriers, f1, families, infeasible)
    }

    /// The F3 price of converting between the two native string carriers across one edge:
    /// `.text` one way, `__rt.convert_host_sb` the other.
    fn f3_conversion(&self, from: Carrier, to: Carrier) -> u64 {
        match (from, to) {
            (Carrier::StringBuilder, Carrier::HostString) => "$cv.text".len() as u64,
            (Carrier::HostString, Carrier::StringBuilder) => {
                "__rt.convert_host_sb($cv)".len() as u64
            }
            _ => INF,
        }
    }

    // -- the commit ----------------------------------------------------------------------------

    /// Builds the whole commit at the given labels, or names the problem nodes whose regions
    /// must roll back.
    fn build_commit(
        &self,
        problem: &Problem,
        carriers: &[Option<Carrier>],
        ops: &BTreeMap<JsName, OpEntry>,
        fault: bool,
    ) -> Result<Commit, (BTreeSet<usize>, String)> {
        let mut commit = Commit::default();
        let native_versions = self.mixed_standard_versions(problem, carriers, ops)?;
        let version_sources: BTreeSet<JsName> = native_versions.keys().cloned().collect();
        let clone_names = self.allocate_clone_names(problem, carriers, &version_sources)?;
        for entry in ops.values() {
            let need = native_versions.get(&entry.name);
            let Some(carrier) = carriers[entry.node].or_else(|| need.map(|need| need.carrier))
            else {
                continue;
            };
            let Some((_, _, rewrite)) = entry.native_at(carrier) else {
                return Err((
                    BTreeSet::from([entry.node]),
                    "the committed carrier has no measured body".to_owned(),
                ));
            };
            if fault {
                let mut failed = BTreeSet::from([entry.node]);
                if let Some(need) = need {
                    failed.extend(need.nodes.iter().copied());
                }
                return Err((failed, "RCGJS_REPR_FAULT".to_owned()));
            }
            let target = clone_names
                .get(&entry.name)
                .cloned()
                .unwrap_or_else(|| entry.name.clone());
            commit.bodies.insert(
                target.clone(),
                NativeBody {
                    parameters: rewrite.parameters.clone(),
                    statements: rewrite.statements.clone(),
                },
            );
            if target != entry.name {
                commit.clones.insert(target, entry.name.clone());
            }
            commit.members.extend(
                rewrite
                    .runtime_helpers
                    .iter()
                    .map(|member| (*member).to_owned()),
            );
        }
        for (name, func) in self.funcs {
            let Ok(func) = func else { continue };
            if ops.contains_key(name) {
                continue;
            }
            match self.commit_func(problem, name, func, carriers, &native_versions, fault) {
                Ok(None) => {
                    // A forwarding wrapper can have a native pointee boundary without carrying
                    // a carrier on any root SSA value. When its baseline address escaped, the
                    // private version is still real: it forwards to the native callee while the
                    // retained source continues to forward to the baseline one.
                    if let Some(target) = clone_names.get(name) {
                        commit.funcs.insert(target.clone(), func.clone());
                        commit.clones.insert(target.clone(), name.clone());
                    } else if native_versions
                        .values()
                        .any(|need| need.callers.contains(name))
                    {
                        commit.funcs.insert(name.clone(), func.clone());
                    }
                }
                Ok(Some((rewritten, adapters, members))) => {
                    commit.adapters += adapters;
                    commit.members.extend(members);
                    let target = clone_names
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.clone());
                    commit.funcs.insert(target.clone(), rewritten);
                    if target != *name {
                        commit.clones.insert(target, name.clone());
                    }
                }
                Err((mut failed, reason)) => {
                    failed.extend(self.nodes_of_item(problem, name, carriers));
                    return Err((failed, reason));
                }
            }
        }
        let native_targets: BTreeMap<ItemId, (JsName, JsName)> = commit
            .clones
            .iter()
            .filter_map(|(clone, source)| {
                let item = self.funcs.get(source)?.as_ref().ok()?.item;
                Some((item, (clone.clone(), source.clone())))
            })
            .collect();
        for (caller, func) in &commit.funcs {
            let mut retargets = crate::lower::CallRetargets::new();
            let source_caller = commit.clones.get(caller).unwrap_or(caller);
            for block in &func.blocks {
                for op in &block.ops {
                    let item = match &op.kind {
                        OpKind::Call {
                            callee: Callee::Direct { item, .. },
                            host: None,
                            ..
                        }
                        | OpKind::Drop {
                            glue: DropGlue::Direct { item, .. },
                            ..
                        } => item,
                        _ => continue,
                    };
                    let Some((target, source)) = native_targets.get(item) else {
                        continue;
                    };
                    // A cut standard body has a private version for a selected set of prepared
                    // callers. Unrelated rewritten functions can still contain baseline calls
                    // to the retained source and must not be swept up by the global item map.
                    if native_versions
                        .get(source)
                        .is_some_and(|need| !need.callers.contains(source_caller))
                    {
                        continue;
                    }
                    retargets.insert(*item, target.clone());
                }
            }
            if !retargets.is_empty() {
                commit.call_retargets.insert(caller.clone(), retargets);
            }
        }
        Ok(commit)
    }

    /// Reference-style standard calls whose caller alone is native. Their compiler tag is the
    /// graph cut: the raw `alloc` implementation stays behind the retained source symbol and the
    /// prepared caller targets one registry-lowered `$n1` body. Value slots use `Convert`
    /// adapters instead, and `StringAsStr` has its cheaper direct call-site interception.
    fn mixed_standard_versions(
        &self,
        problem: &Problem,
        carriers: &[Option<Carrier>],
        ops: &BTreeMap<JsName, OpEntry>,
    ) -> Result<BTreeMap<JsName, NativeVersionNeed>, CommitDecline> {
        let mut needs: BTreeMap<JsName, NativeVersionNeed> = BTreeMap::new();
        // Retargets are deliberately one-per-caller-and-callee: the private ABI name is a link
        // decision, not an extra field serialized into every JIR call. That is only sound when
        // every call site to that standard item in one emitted caller wants the same ABI. Keep
        // the site states here and roll the native region back if a caller mixes them.
        let mut baseline_sites: BTreeSet<(JsName, JsName)> = BTreeSet::new();
        for (caller_name, caller_func) in self.funcs {
            let Ok(caller_func) = caller_func else {
                continue;
            };
            if self.is_candidate_glue_or_standard(caller_name) {
                continue;
            }
            for block in &caller_func.blocks {
                for op in &block.ops {
                    let OpKind::Call {
                        callee: Callee::Direct { item, .. },
                        args,
                        semantic: CallSemantic::Standard(operation),
                        host: None,
                        ..
                    } = &op.kind
                    else {
                        continue;
                    };
                    if *operation == StandardOperation::StringAsStr {
                        continue;
                    }
                    let Some(registry) = semantic_ops::registry(*operation) else {
                        continue;
                    };
                    let Some(callee_name) = self.lowered.get(item) else {
                        continue;
                    };
                    let (Some(op_entry), Some(Ok(callee_func))) =
                        (ops.get(callee_name), self.funcs.get(callee_name))
                    else {
                        continue;
                    };
                    if !call_aligns(callee_func, args.len()) {
                        continue;
                    }
                    let mut site_native = BTreeSet::new();
                    let mut site_baseline = false;
                    for requirement in registry
                        .slots
                        .iter()
                        .filter(|requirement| requirement.reference != ReferenceRequirement::Value)
                    {
                        let caller_node = site_slot_key(caller_name, args, op.result, requirement)
                            .and_then(|key| problem.resolution.get(&key))
                            .copied();
                        let Some(caller_node) = caller_node else {
                            site_baseline = true;
                            continue;
                        };
                        let Some(near) = carriers[caller_node] else {
                            site_baseline = true;
                            continue;
                        };
                        let far = self
                            .slot_node(problem, callee_name, callee_func, requirement)
                            .and_then(|node| carriers[node]);
                        if far.is_some() {
                            continue;
                        }
                        if op_entry.native_at(near).is_none() {
                            return Err((
                                BTreeSet::from([caller_node]),
                                "legality.no-carrier".to_owned(),
                            ));
                        }
                        site_native.insert((caller_node, near));
                    }
                    if site_native.is_empty() {
                        if site_baseline {
                            baseline_sites.insert((caller_name.clone(), callee_name.clone()));
                        }
                        continue;
                    }
                    if site_baseline {
                        return Err((
                            site_native.iter().map(|(node, _)| *node).collect(),
                            "version.mixed-call-sites".to_owned(),
                        ));
                    }
                    for (caller_node, near) in site_native {
                        match needs.entry(callee_name.clone()) {
                            std::collections::btree_map::Entry::Vacant(slot) => {
                                slot.insert(NativeVersionNeed {
                                    carrier: near,
                                    callers: BTreeSet::from([caller_name.clone()]),
                                    nodes: BTreeSet::from([caller_node]),
                                });
                            }
                            std::collections::btree_map::Entry::Occupied(mut slot) => {
                                let need = slot.get_mut();
                                if need.carrier != near {
                                    let mut failed = need.nodes.clone();
                                    failed.insert(caller_node);
                                    return Err((
                                        failed,
                                        versions::budget(2)
                                            .expect_err("two carriers require two versions")
                                            .code()
                                            .to_owned(),
                                    ));
                                }
                                need.callers.insert(caller_name.clone());
                                need.nodes.insert(caller_node);
                            }
                        }
                    }
                }
            }
        }
        for (callee, need) in &needs {
            if need
                .callers
                .iter()
                .any(|caller| baseline_sites.contains(&(caller.clone(), callee.clone())))
            {
                return Err((need.nodes.clone(), "version.mixed-call-sites".to_owned()));
            }
        }
        Ok(needs)
    }

    /// Allocates one private name for every native-labeled item boundary whose baseline body is
    /// retained, then closes that set over native direct callees. The closure matters for a
    /// forwarding wrapper: its retained body must keep calling baseline callees while its private
    /// version targets their native versions. Only prepared JIR callers are retargeted after the
    /// transactional commit succeeds.
    fn allocate_clone_names(
        &self,
        problem: &Problem,
        carriers: &[Option<Carrier>],
        additional_sources: &BTreeSet<JsName>,
    ) -> Result<BTreeMap<JsName, JsName>, CommitDecline> {
        if std::env::var("RCGJS_REPR_DEBUG_VERSIONS").is_ok() {
            for key in &self.graph.pinned_boundaries {
                let label = problem.resolution.get(key).and_then(|node| carriers[*node]);
                eprintln!("repr-solve: version-boundary {key} = {label:?}");
            }
        }
        let native_items: BTreeSet<JsName> = problem
            .resolution
            .iter()
            .filter_map(|(key, node)| {
                carriers[*node]?;
                match key {
                    NodeKey::Value { item, .. } => Some(item.clone()),
                    NodeKey::Baseline
                    | NodeKey::Group { .. }
                    | NodeKey::Opaque { .. }
                    | NodeKey::Static { .. } => None,
                }
            })
            .collect();
        let mut sources: BTreeSet<JsName> = self
            .graph
            .pinned_boundaries
            .iter()
            .filter_map(|key| {
                let node = *problem.resolution.get(key)?;
                carriers[node]?;
                match key {
                    NodeKey::Value { item, .. } => Some(item.clone()),
                    NodeKey::Baseline
                    | NodeKey::Group { .. }
                    | NodeKey::Opaque { .. }
                    | NodeKey::Static { .. } => None,
                }
            })
            .collect();
        sources.extend(additional_sources.iter().cloned());
        let mut queue: VecDeque<JsName> = sources.iter().cloned().collect();
        while let Some(source) = queue.pop_front() {
            let Some(Ok(func)) = self.funcs.get(&source) else {
                continue;
            };
            for block in &func.blocks {
                for op in &block.ops {
                    let item = match &op.kind {
                        OpKind::Call {
                            callee: Callee::Direct { item, .. },
                            host: None,
                            ..
                        }
                        | OpKind::Drop {
                            glue: DropGlue::Direct { item, .. },
                            ..
                        } => Some(item),
                        _ => None,
                    };
                    let Some(callee) = item.and_then(|item| self.lowered.get(item)) else {
                        continue;
                    };
                    if native_items.contains(callee) && sources.insert(callee.clone()) {
                        queue.push_back(callee.clone());
                    }
                }
            }
        }
        let mut taken: BTreeSet<String> = self
            .table
            .keys()
            .map(|name| name.as_str().to_owned())
            .chain(
                self.table
                    .values()
                    .flat_map(|item| item.refs.iter().map(|name| name.as_str().to_owned())),
            )
            .collect();
        let mut names = BTreeMap::new();
        for source in sources {
            versions::budget(1).map_err(|decline| {
                (
                    self.nodes_of_item(problem, &source, carriers),
                    decline.code().to_owned(),
                )
            })?;
            let name = versions::allocate_version_name_owned(source.as_str(), &taken).map_err(
                |decline| {
                    (
                        self.nodes_of_item(problem, &source, carriers),
                        decline.code().to_owned(),
                    )
                },
            )?;
            taken.insert(name.as_str().to_owned());
            names.insert(source, name);
        }
        Ok(names)
    }

    /// The labeled problem nodes anchored in one item.
    fn nodes_of_item(
        &self,
        problem: &Problem,
        name: &JsName,
        carriers: &[Option<Carrier>],
    ) -> BTreeSet<usize> {
        problem
            .resolution
            .iter()
            .filter(|(key, index)| {
                matches!(key, NodeKey::Value { item, .. } if item == name)
                    && carriers[**index].is_some()
            })
            .map(|(_, index)| *index)
            .collect()
    }

    /// Rewrites one ordinary `Func` at the labels — carriers on its values, a `Convert` at every
    /// cut boundary — and verifies it under exactly the committed policy. `Ok(None)` = the
    /// labels do not touch this function.
    fn commit_func(
        &self,
        problem: &Problem,
        name: &JsName,
        func: &Func,
        carriers: &[Option<Carrier>],
        native_versions: &BTreeMap<JsName, NativeVersionNeed>,
        fault: bool,
    ) -> Result<Option<(Func, u32, BTreeSet<String>)>, CommitDecline> {
        let label = |item: &JsName, site: Site, path: &[Step]| -> Option<Carrier> {
            problem
                .resolution
                .get(&NodeKey::Value {
                    item: item.clone(),
                    site,
                    path: path.to_vec(),
                })
                .and_then(|index| carriers[*index])
        };

        let mut out = func.clone();
        let mut touched = false;
        for index in 0..out.values.len() {
            if let Some(carrier) = label(name, Site::Value(ValueId(index as u32)), &[]) {
                out.values[index].carrier = carrier;
                touched = true;
            }
        }

        let mut adapters = 0u32;
        let mut members: BTreeSet<String> = BTreeSet::new();
        let interns = self.interns.get(name).ok_or_else(|| {
            (
                BTreeSet::new(),
                format!("`{}` has no interns", name.as_str()),
            )
        })?;
        let types = &interns.types;

        // The proven format sites (W4g): a site whose participants all labeled native commits
        // by marking the constructor calls' results — the in-band signal the emission hook in
        // `lower/js/call.rs` keys on — and keeps its calls out of the generic boundary handling
        // below (the inlined site spells its own reads). A site labeled baseline throughout
        // stays plain calls to the baseline machinery; a mixed labeling rolls the region back.
        let mut fmt_elide: BTreeSet<(usize, usize)> = BTreeSet::new();
        if let Some(analysis) = self.fmt.get(name) {
            use semantic_ops::fmt_kernel::{DisplayKind, SiteKind};
            let native = |carrier: Option<Carrier>| {
                matches!(carrier, Some(Carrier::HostString | Carrier::StringBuilder))
            };
            for site in &analysis.sites {
                // A callee an unproven call still reaches kept its edges; its sites stay plain.
                if !site
                    .callees
                    .iter()
                    .all(|item| analysis.elidable.contains(item))
                {
                    continue;
                }
                let dest = match site.kind {
                    SiteKind::ToString { result } => label(name, Site::Value(result), &[]),
                    SiteKind::WriteFmt { receiver, .. } => {
                        label(name, Site::Value(receiver), &[Step::Deref])
                    }
                };
                let mut strings: Vec<(ValueId, Option<Carrier>)> = Vec::new();
                for arg in &site.args {
                    if arg.kind != DisplayKind::String {
                        continue;
                    }
                    let value_site = match arg.root {
                        Root::Value(v) => Site::Value(v),
                        Root::Cell(c) => Site::Cell(c),
                    };
                    let steps: Vec<Step> = arg.path.iter().map(|_| Step::Deref).collect();
                    strings.push((arg.value, label(name, value_site, &steps)));
                }
                let all = native(dest) && strings.iter().all(|(_, l)| native(*l));
                let any = native(dest) || strings.iter().any(|(_, l)| native(*l));
                if !any {
                    continue;
                }
                if !all {
                    return Err((
                        BTreeSet::new(),
                        format!("`{}` format site solved to mixed carriers", name.as_str()),
                    ));
                }
                if matches!(site.kind, SiteKind::WriteFmt { .. })
                    && dest != Some(Carrier::StringBuilder)
                {
                    return Err((
                        BTreeSet::new(),
                        format!("`{}` format receiver is not a StringBuilder", name.as_str()),
                    ));
                }
                out.values[site.args_value.0 as usize].carrier = Carrier::HostString;
                for arg in &site.args {
                    out.values[arg.value.0 as usize].carrier = match arg.kind {
                        // The mark records the source's own committed carrier, so the emission
                        // reads `.text` exactly when the source is a builder.
                        DisplayKind::String => strings
                            .iter()
                            .find(|(value, _)| *value == arg.value)
                            .and_then(|(_, label)| *label)
                            .unwrap_or(Carrier::HostString),
                        DisplayKind::F64 => {
                            members.insert("native_f64_to_string".to_owned());
                            Carrier::HostString
                        }
                        _ => Carrier::HostString,
                    };
                }
                if matches!(site.kind, SiteKind::WriteFmt { .. })
                    || dest == Some(Carrier::StringBuilder)
                {
                    members.insert("str_len".to_owned());
                }
                fmt_elide.insert(site.consumer);
                fmt_elide.insert(site.new_call);
                fmt_elide.extend(site.display_calls.iter().copied());
                touched = true;
            }
        }

        for block_index in 0..out.blocks.len() {
            let taken = std::mem::take(&mut out.blocks[block_index].ops);
            let mut rebuilt = Vec::with_capacity(taken.len());
            for (op_index, mut op) in taken.into_iter().enumerate() {
                if fmt_elide.contains(&(block_index, op_index)) {
                    // An inlined format call: the emission hook spells it whole; no generic
                    // boundary conversion may touch it.
                    rebuilt.push(op);
                    continue;
                }
                let origin = op.origin;
                match &op.kind {
                    OpKind::Call {
                        callee,
                        args,
                        semantic,
                        host,
                        ..
                    } if host.is_none() && *semantic != CallSemantic::Host => {
                        // The D2 cut's soundness net at reference slots: a registry-covered
                        // standard call shares storage through its reference requirements, which
                        // no `Convert` can spell, so the caller's label behind the deref and the
                        // callee's must agree — a disagreement rolls both regions back (the
                        // error names both items' labeled nodes).
                        if let (CallSemantic::Standard(operation), Callee::Direct { item, .. }) =
                            (semantic, callee)
                        {
                            if let (Some(entry), Some(callee_name)) =
                                (semantic_ops::registry(*operation), self.lowered.get(item))
                            {
                                if let Some(Ok(callee_func)) = self.funcs.get(callee_name) {
                                    if call_aligns(callee_func, args.len()) {
                                        for requirement in entry.slots {
                                            if requirement.reference == ReferenceRequirement::Value
                                            {
                                                continue;
                                            }
                                            let SignatureSlot::Argument(index) = requirement.slot
                                            else {
                                                continue;
                                            };
                                            let index = usize::from(index);
                                            let Some(arg) = args.get(index) else {
                                                continue;
                                            };
                                            let near =
                                                label(name, Site::Value(*arg), &[Step::Deref])
                                                    .unwrap_or(Carrier::Baseline);
                                            let far = callee_func
                                                .params
                                                .get(index)
                                                .and_then(|param| {
                                                    label(
                                                        callee_name,
                                                        Site::Value(*param),
                                                        &[Step::Deref],
                                                    )
                                                })
                                                .unwrap_or(Carrier::Baseline);
                                            let callsite_cut = *operation
                                                == StandardOperation::StringAsStr
                                                && matches!(
                                                    near,
                                                    Carrier::HostString | Carrier::StringBuilder
                                                );
                                            let versioned_cut = native_versions
                                                .get(callee_name)
                                                .is_some_and(|need| need.carrier == near);
                                            if near != far && !callsite_cut && !versioned_cut {
                                                let mut failed =
                                                    self.nodes_of_item(problem, name, carriers);
                                                failed.extend(self.nodes_of_item(
                                                    problem,
                                                    callee_name,
                                                    carriers,
                                                ));
                                                return Err((
                                                    failed,
                                                    format!(
                                                        "`{}` cut standard call to `{}` \
                                                         disagrees at reference slot {index}",
                                                        name.as_str(),
                                                        callee_name.as_str()
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // The far side's slot labels: a lowered direct callee answers per slot;
                        // an unlowered, foreign or runtime callee is all baseline; an indirect
                        // or virtual call is never cut (its arcs are priced INF).
                        let far = match callee {
                            Callee::Direct { item, .. } => {
                                // Exactly `valueflow::connect_call`'s alignment rule, the
                                // trailing `$loc` of a `#[track_caller]` callee included: the
                                // commit must model the same far side the solve labeled.
                                let lowered = self.lowered.get(item).and_then(|callee_name| {
                                    match self.funcs.get(callee_name) {
                                        Some(Ok(callee_func))
                                            if callee_func.sig.abi.spread.is_none()
                                                && (callee_func.params.len() == args.len()
                                                    || (callee_func.sig.abi.caller_location
                                                        && callee_func.params.len()
                                                            == args.len() + 1)) =>
                                        {
                                            Some((callee_name.clone(), callee_func.params.clone()))
                                        }
                                        _ => None,
                                    }
                                });
                                Some(lowered)
                            }
                            Callee::Foreign { .. } | Callee::Runtime { .. } => Some(None),
                            _ => None,
                        };
                        let Some(far) = far else {
                            rebuilt.push(op);
                            continue;
                        };
                        let arg_values: Vec<ValueId> = args.clone();
                        let mut new_args = arg_values.clone();
                        for (position, arg) in arg_values.iter().enumerate() {
                            let al = out.values[arg.0 as usize].carrier;
                            let fl = far
                                .as_ref()
                                .and_then(|(callee_name, params)| {
                                    label(callee_name, Site::Value(*params.get(position)?), &[])
                                })
                                .unwrap_or(Carrier::Baseline);
                            if al == fl {
                                continue;
                            }
                            let ty = out.values[arg.0 as usize].ty;
                            let converted = push_value(&mut out.values, ty, fl, origin);
                            rebuilt.push(Op {
                                result: Some(converted),
                                origin,
                                kind: OpKind::Convert {
                                    v: *arg,
                                    from: al,
                                    to: fl,
                                },
                            });
                            new_args[position] = converted;
                            adapters += 1;
                            touched = true;
                            self.conversion_members(types, interns, ty, al, fl, &mut members);
                        }
                        let result_fix = op.result.and_then(|result| {
                            let rl = out.values[result.0 as usize].carrier;
                            let frl = far
                                .as_ref()
                                .and_then(|(callee_name, _)| label(callee_name, Site::Ret, &[]))
                                .unwrap_or(Carrier::Baseline);
                            (rl != frl).then_some((result, frl, rl))
                        });
                        if let OpKind::Call { args, .. } = &mut op.kind {
                            *args = new_args;
                        }
                        match result_fix {
                            None => rebuilt.push(op),
                            Some((result, from, to)) => {
                                // The call defines a fresh value in the callee's shape; the
                                // convert re-defines the original id, so every use stands.
                                let ty = out.values[result.0 as usize].ty;
                                let fresh = push_value(&mut out.values, ty, from, origin);
                                op.result = Some(fresh);
                                rebuilt.push(op);
                                rebuilt.push(Op {
                                    result: Some(result),
                                    origin,
                                    kind: OpKind::Convert { v: fresh, from, to },
                                });
                                adapters += 1;
                                touched = true;
                                self.conversion_members(types, interns, ty, from, to, &mut members);
                            }
                        }
                    }
                    OpKind::Drop { root, path, glue } if path.is_empty() => {
                        let al = match root {
                            Root::Value(v) => out.values[v.0 as usize].carrier,
                            Root::Cell(c) => {
                                label(name, Site::Cell(*c), &[]).unwrap_or(Carrier::Baseline)
                            }
                        };
                        let gl = match glue {
                            DropGlue::Direct { item, .. } => self
                                .lowered
                                .get(item)
                                .and_then(|glue_name| {
                                    let glue_func =
                                        self.funcs.get(glue_name).and_then(|f| f.as_ref().ok())?;
                                    let receiver = *glue_func.params.first()?;
                                    // A by-reference glue labels its receiver behind the deref,
                                    // a by-value one at the value itself.
                                    label(glue_name, Site::Value(receiver), &[Step::Deref])
                                        .or_else(|| label(glue_name, Site::Value(receiver), &[]))
                                })
                                .unwrap_or(Carrier::Baseline),
                            DropGlue::None | DropGlue::Dynamic => Carrier::Baseline,
                        };
                        if al == gl {
                            rebuilt.push(op);
                            continue;
                        }
                        let glue = *glue;
                        let (dropped, ty) = match root {
                            Root::Value(v) => (*v, out.values[v.0 as usize].ty),
                            Root::Cell(c) => {
                                let cell = out.cells[c.0 as usize];
                                if cell.boxed {
                                    // NativeBox deliberately keeps the baseline pointer shape.
                                    // A boxed local can therefore cross this ownership-only cut
                                    // without materializing or rewriting its storage; keeping the
                                    // Drop op lets the retarget pass select the native glue.
                                    if matches!(
                                        (al, gl),
                                        (Carrier::Baseline, Carrier::NativeBox)
                                            | (Carrier::NativeBox, Carrier::Baseline)
                                    ) {
                                        rebuilt.push(op);
                                        touched = true;
                                        continue;
                                    }
                                    return Err((
                                        BTreeSet::new(),
                                        format!(
                                            "`{}` drops a boxed cell across a carrier cut",
                                            name.as_str()
                                        ),
                                    ));
                                }
                                let value = push_value(&mut out.values, cell.ty, al, origin);
                                rebuilt.push(Op {
                                    result: Some(value),
                                    origin,
                                    kind: OpKind::CellLoad(*c),
                                });
                                (value, cell.ty)
                            }
                        };
                        let converted = push_value(&mut out.values, ty, gl, origin);
                        rebuilt.push(Op {
                            result: Some(converted),
                            origin,
                            kind: OpKind::Convert {
                                v: dropped,
                                from: al,
                                to: gl,
                            },
                        });
                        rebuilt.push(Op {
                            result: None,
                            origin,
                            kind: OpKind::Drop {
                                root: Root::Value(converted),
                                path: Vec::new(),
                                glue,
                            },
                        });
                        adapters += 1;
                        touched = true;
                        self.conversion_members(types, interns, ty, al, gl, &mut members);
                    }
                    _ => rebuilt.push(op),
                }
            }
            out.blocks[block_index].ops = rebuilt;
        }

        if !touched {
            return Ok(None);
        }
        let policy = out
            .values
            .iter()
            .fold(CarrierPolicy::BASELINE, |policy, def| {
                policy.with(def.carrier)
            });
        out.verify_carriers(types, policy)
            .map_err(|err| (BTreeSet::new(), format!("`{}`: {err}", name.as_str())))?;
        if fault {
            return Err((
                BTreeSet::new(),
                format!("`{}`: RCGJS_REPR_FAULT", name.as_str()),
            ));
        }
        Ok(Some((out, adapters, members)))
    }

    /// Adds the shim members one committed conversion's spelling reaches.
    fn conversion_members(
        &self,
        table: &TypeTable,
        interns: &Interns,
        ty: TypeId,
        from: Carrier,
        to: Carrier,
        members: &mut BTreeSet<String>,
    ) {
        if let Ok(expr) = adapters::convert(table, interns, jsast::id("$cv"), ty, from, to) {
            let stmt = jsast::Stmt::ExprStmt(expr);
            let printed = jsast::program_to_string_styled_for_target(
                std::slice::from_ref(&stmt),
                jsast::Style::Compact,
                crate::opts::get().ecma_target,
            );
            members.extend(rt_members(&printed));
        }
    }

    // -- the accounting ------------------------------------------------------------------------

    /// The sidecar's `repr` block: D12's gate arithmetic over the regions.
    #[allow(clippy::too_many_arguments)] // The block is a summary of exactly these inputs.
    fn account(
        &self,
        problem: &Problem,
        regions: &[BTreeSet<usize>],
        carriers: &[Option<Carrier>],
        pairs: &[Pair],
        unaries: &(Vec<u64>, Vec<u64>),
        demoted: &BTreeMap<usize, String>,
        rollbacks: &BTreeMap<usize, String>,
        no_carrier: &BTreeSet<usize>,
        proposed: &BTreeSet<usize>,
        commit: &Commit,
        families: u32,
    ) -> ReprBlock {
        let forced = forced_closure(problem, pairs, &unaries.1);
        // The problem nodes carrying an item-boundary pin (`valueflow::pins`): what D10b's
        // `pinned_cost_positive` accounting counts regions blocked by.
        let pinned_nodes: BTreeSet<usize> = self
            .graph
            .pinned_boundaries
            .iter()
            .filter_map(|key| problem.resolution.get(key))
            .copied()
            .collect();
        let mut block = ReprBlock {
            families,
            adapters: commit.adapters,
            versions: commit.clones.len() as u32,
            emits: true,
            ..ReprBlock::default()
        };
        if let Some((before, after)) = commit.measured {
            block.bytes_before = before.bytes;
            block.bytes_after = after.bytes;
            block.score_before = before.total();
            block.score_after = after.total();
        }
        for member in cost::helper_closure(&commit.members) {
            if !self.reached.contains(&member) {
                block.helper_bytes += self.member_cost(&member);
            }
        }
        for region in regions {
            let representative = *region.iter().next().expect("a region has members");
            let family = region
                .iter()
                .find_map(|node| problem.nodes[*node].family)
                .expect("a candidate region has a family");
            let family_carriers: &[&str] = match family {
                CandidateKind::String => &["host-string", "string-builder"],
                CandidateKind::Vec => &["native-vec", "slice-view"],
                CandidateKind::Box => &["native-box"],
            };
            let impossible = region.iter().any(|node| no_carrier.contains(node));
            let legal = !impossible && region.iter().any(|node| !forced[*node]);
            let native = region.iter().find_map(|node| carriers[*node]);
            block.candidates += 1;
            if legal {
                block.legal += 1;
            }
            let mut decision = ReprDecision {
                subject: problem.nodes[representative].key.to_string(),
                family: "carrier".to_owned(),
                chosen: "baseline".to_owned(),
                alternatives: std::iter::once("baseline")
                    .chain(family_carriers.iter().copied())
                    .map(str::to_owned)
                    .collect(),
                decline: None,
                items: region
                    .iter()
                    .filter_map(|node| match &problem.nodes[*node].key {
                        ProblemKey::Value(NodeKey::Value { item, .. }) => Some(item.clone()),
                        _ => None,
                    })
                    .collect(),
            };
            if region.iter().any(|node| proposed.contains(node)) {
                block.planned += 1;
            }
            if native.is_none() {
                // D10b: a region whose ONLY infinite reasons are item-boundary pins
                // (`unknown-call` from `valueflow::pins`) is what one `$n` clone would free;
                // the count is the clone path's build-or-record evidence (W5g reads it).
                let mut pinned = false;
                let mut pin_blocked_only = true;
                for node in region {
                    let inf = &problem.nodes[*node].inf;
                    if inf.is_empty() {
                        continue;
                    }
                    if pinned_nodes.contains(node)
                        && inf
                            .iter()
                            .all(|reason| *reason == legality::InfReason::UnknownCall)
                    {
                        pinned = true;
                    } else {
                        pin_blocked_only = false;
                    }
                }
                if pinned && pin_blocked_only {
                    block.pinned_cost_positive += 1;
                }
            }
            if impossible {
                decision.decline = Some("legality.no-carrier".to_owned());
            } else if let Some(carrier) = native {
                block.cost_positive += 1;
                block.committed += 1;
                decision.family = match family {
                    CandidateKind::String => "string",
                    CandidateKind::Vec => "vec",
                    CandidateKind::Box => "carrier",
                }
                .to_owned();
                decision.chosen = carrier_name(carrier).to_owned();
                decision.alternatives = family_carriers.iter().map(|c| (*c).to_owned()).collect();
            } else if let Some(reason) = region.iter().find_map(|node| rollbacks.get(node)) {
                // Solved cost-positive, failed the transaction.
                block.cost_positive += 1;
                decision.decline = Some(reason.clone());
            } else if let Some(reason) = region.iter().find_map(|node| demoted.get(node)) {
                decision.decline = Some(reason.clone());
            } else if legal {
                decision.decline =
                    Some(Decline::Cost(CostDecline::RegionNoShrink).code().to_owned());
            } else {
                let reason = region
                    .iter()
                    .flat_map(|node| problem.nodes[*node].inf.iter())
                    .next()
                    .map(|reason| reason.as_str())
                    .unwrap_or("must-equal-baseline");
                decision.decline = Some(format!("legality.{reason}"));
            }
            block.decisions.push(decision);
        }
        block.applied = block.committed;
        // One row per exported candidate slot. Export wrappers are not implemented yet, so the
        // `Exported` legality pin keeps every public row at the baseline ABI.
        for key in &self.graph.exported_slots {
            let Some(node) = problem.resolution.get(key) else {
                continue;
            };
            let retained = matches!(key, NodeKey::Value { item, .. }
                if commit.clones.values().any(|source| source == item));
            let chosen = if retained {
                "baseline"
            } else {
                carriers[*node].map(carrier_name).unwrap_or("baseline")
            };
            block.exports.push(ReprExport {
                subject: key.to_string(),
                chosen: chosen.to_owned(),
            });
        }
        block
    }
}

/// Whether a call's arguments align positionally with the callee's parameters — exactly
/// `valueflow`'s rule, the trailing `$loc` of a `#[track_caller]` callee included.
fn call_aligns(callee: &Func, args: usize) -> bool {
    callee.sig.abi.spread.is_none()
        && (callee.params.len() == args
            || (callee.sig.abi.caller_location && callee.params.len() == args + 1))
}

/// The caller-side node of one slot requirement at one call site: the twin of
/// [`Planner::slot_node`] mapped onto the site's arguments and result.
fn site_slot_key(
    name: &JsName,
    args: &[ValueId],
    result: Option<ValueId>,
    requirement: &SlotRequirement,
) -> Option<NodeKey> {
    let (value, path) = match requirement.slot {
        SignatureSlot::Result => (result?, Vec::new()),
        SignatureSlot::Argument(index) => {
            let value = *args.get(usize::from(index))?;
            let path = if requirement.reference == ReferenceRequirement::Value {
                Vec::new()
            } else {
                vec![Step::Deref]
            };
            (value, path)
        }
    };
    Some(NodeKey::Value {
        item: name.clone(),
        site: Site::Value(value),
        path,
    })
}

/// A fresh value definition.
fn push_value(
    values: &mut Vec<ValueDef>,
    ty: TypeId,
    carrier: Carrier,
    origin: crate::jir::origin::OriginId,
) -> ValueId {
    let id = ValueId(values.len() as u32);
    values.push(ValueDef {
        ty,
        carrier,
        origin,
    });
    id
}

/// The nodes a native label is structurally forbidden for: INF unaries closed over the INF arcs
/// (a `b` that cannot be native beside forbids `a` wherever `a` native beside `b` baseline costs
/// INF).
fn forced_closure(problem: &Problem, pairs: &[Pair], native_costs: &[u64]) -> Vec<bool> {
    let mut forced: Vec<bool> = native_costs.iter().map(|cost| *cost >= INF).collect();
    forced[problem.baseline] = true;
    loop {
        let mut changed = false;
        for pair in pairs {
            if forced[pair.b] && pair.ab >= INF && !forced[pair.a] {
                forced[pair.a] = true;
                changed = true;
            }
            if forced[pair.a] && pair.ba >= INF && !forced[pair.b] {
                forced[pair.b] = true;
                changed = true;
            }
        }
        if !changed {
            return forced;
        }
    }
}

/// The connected components of the cut's native side, over every arc with both ends native:
/// the unit the measured Δ gate demotes by.
fn native_components(pairs: &[Pair], native_side: &[bool]) -> Vec<BTreeSet<usize>> {
    let mut unions = UnionFind::new(native_side.len());
    for pair in pairs {
        if native_side[pair.a] && native_side[pair.b] {
            unions.union(pair.a, pair.b);
        }
    }
    let mut by_root: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (index, native) in native_side.iter().enumerate() {
        if *native {
            by_root.entry(unions.root(index)).or_default().insert(index);
        }
    }
    by_root.into_values().collect()
}

/// The exact D9 refusal recorded for a measured vector that cannot be accepted.
fn exact_cost_decline(
    economics: CarrierEconomics,
    baseline: ProgramCost,
    candidate: Option<ProgramCost>,
) -> String {
    let before = baseline.total();
    let Some(candidate) = candidate else {
        return format!(
            "{} policy={} before={before} after=INF",
            Decline::Cost(CostDecline::NoImprovement { before, after: INF }).code(),
            economics.as_str()
        );
    };
    let after = candidate.total();
    let growth = if candidate.emitted_bytes() > baseline.emitted_bytes() {
        Some("compiler-link-bytes")
    } else if candidate.gzip > baseline.gzip {
        Some("compiler-link-gzip")
    } else if candidate.brotli > baseline.brotli {
        Some("compiler-link-brotli")
    } else if !matches!(economics, CarrierEconomics::Size)
        && candidate.allocations > baseline.allocations
    {
        Some("final-js-allocations")
    } else if matches!(
        economics,
        CarrierEconomics::Balanced | CarrierEconomics::Speed
    ) && candidate.loop_allocations > baseline.loop_allocations
    {
        Some("final-js-loop-allocations")
    } else {
        None
    };
    if let Some(budget) = growth {
        return format!(
            "{} policy={} zero-growth={} raw={}->{} gzip={}->{} brotli={}->{} allocations={}->{} loop_allocations={}->{}",
            Decline::Cost(CostDecline::Budget {
                budget: budget.to_owned(),
            })
            .code(),
            economics.as_str(),
            budget,
            baseline.emitted_bytes(),
            candidate.emitted_bytes(),
            baseline.gzip,
            candidate.gzip,
            baseline.brotli,
            candidate.brotli,
            baseline.allocations,
            candidate.allocations,
            baseline.loop_allocations,
            candidate.loop_allocations,
        );
    }
    if before == after {
        format!(
            "{} policy={} old={before} new={after}",
            Decline::Cost(CostDecline::RegionNoShrinkBytes {
                members: Vec::new(),
                old_bytes: before,
                new_bytes: after,
            })
            .code(),
            economics.as_str(),
        )
    } else {
        format!(
            "{} policy={} before={before} after={after} raw={}->{} loop_allocations={}->{}",
            Decline::Cost(CostDecline::NoImprovement { before, after }).code(),
            economics.as_str(),
            baseline.emitted_bytes(),
            candidate.emitted_bytes(),
            baseline.loop_allocations,
            candidate.loop_allocations,
        )
    }
}

/// The must-equal-connected components of the problem's candidate value nodes, the anchor and
/// the facility nodes excluded: the D12 region a decision row is written about. A host pin does
/// not merge its endpoint into the anchor — it forces the label; the row stays the value's own.
fn regions(problem: &Problem, terms: &[Term]) -> Vec<BTreeSet<usize>> {
    let count = problem.nodes.len();
    let mut unions = UnionFind::new(count);
    for term in terms {
        let Term::MustEqual { a, b, reason } = term else {
            continue;
        };
        if *reason == EqualReason::HostPin {
            continue;
        }
        let (Some(a), Some(b)) = (
            problem.resolution.get(a).copied(),
            problem.resolution.get(b).copied(),
        ) else {
            continue;
        };
        if a != problem.baseline && b != problem.baseline {
            unions.union(a, b);
        }
    }
    let mut by_root: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for index in 0..count {
        if index == problem.baseline
            || matches!(
                problem.nodes[index].key,
                ProblemKey::Version { .. } | ProblemKey::Helper(_)
            )
        {
            continue;
        }
        by_root.entry(unions.root(index)).or_default().insert(index);
    }
    by_root
        .into_values()
        .filter(|members| {
            members
                .iter()
                .any(|index| problem.nodes[*index].family.is_some())
        })
        .collect()
}

/// The alias-anchor soundness guard: a region whose ordinary-item members are all pure aliases
/// (every path steps through a `deref` or an element) has no modeled owner for the storage those
/// aliases point into — a `&String` read out of an `Rc<[String]>` element is the canonical case:
/// discovery never unioned the alias to the storage it reads, so committing the region would
/// change the alias's expected shape while the storage keeps producing baseline values. Such a
/// region is forced baseline with [`InfReason::UnsupportedOperation`] (an operation discovery
/// could not model defined the alias). A region with any owned (empty-path) ordinary member is
/// anchored and passes; a region made only of committed-operation slots needs no anchor — its
/// boundaries are adapter edges the commit spells explicitly.
fn anchor_guard(
    problem: &mut Problem,
    regions: &[BTreeSet<usize>],
    ops: &BTreeMap<JsName, OpEntry>,
) {
    use crate::analysis::legality::InfReason;
    let mut members: BTreeMap<usize, Vec<NodeKey>> = BTreeMap::new();
    for (key, index) in &problem.resolution {
        members.entry(*index).or_default().push(key.clone());
    }
    let op_items: BTreeSet<&JsName> = ops.keys().collect();
    let ordinary = |key: &NodeKey, want_owned: bool| {
        matches!(key, NodeKey::Value { item, path, .. }
            if !op_items.contains(item) && path.is_empty() == want_owned)
    };
    for region in regions {
        let of = |want_owned: bool| {
            region.iter().any(|node| {
                members
                    .get(node)
                    .is_some_and(|keys| keys.iter().any(|key| ordinary(key, want_owned)))
            })
        };
        if of(true) || !of(false) {
            continue;
        }
        for node in region {
            problem.nodes[*node]
                .inf
                .insert(InfReason::UnsupportedOperation);
        }
    }
}

fn carrier_name(carrier: Carrier) -> &'static str {
    match carrier {
        Carrier::Baseline => "baseline",
        Carrier::Virtual => "virtual",
        Carrier::Flat => "flat",
        Carrier::HostString => "host-string",
        Carrier::StringBuilder => "string-builder",
        Carrier::NativeVec => "native-vec",
        Carrier::SliceView => "slice-view",
        Carrier::Opaque => "opaque",
        Carrier::NativeBox => "native-box",
    }
}

/// The proven format sites of every ordinary lowered item (W4g), keyed by consumer.
///
/// A site whose plan keys the value graph cannot resolve un-proves the whole item — its
/// constructor calls would be left without a committed consumer — which is exactly the
/// decline-whole-item rule the kernel inherits from v1.
fn fmt_analyses(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns_map: &BTreeMap<JsName, &Interns>,
    lowered: &BTreeMap<ItemId, JsName>,
    graph: &ValueGraph,
) -> BTreeMap<JsName, semantic_ops::fmt_kernel::Analysis> {
    let resolve = |item: &ItemId| -> Option<StandardOperation> {
        let name = lowered.get(item)?;
        match table.get(name)?.jir.body.as_ref()?.semantic {
            BodySemantic::Standard(operation) => Some(operation),
            _ => None,
        }
    };
    let mut out = BTreeMap::new();
    for (name, func) in funcs {
        let Ok(func) = func else { continue };
        if !matches!(
            table
                .get(name)
                .and_then(|item| item.jir.body.as_ref())
                .map(|body| body.semantic),
            Some(BodySemantic::Ordinary)
        ) {
            continue;
        }
        let Some(interns) = interns_map.get(name) else {
            continue;
        };
        let analysis = semantic_ops::fmt_kernel::analyze(func, &interns.types, interns, &resolve);
        if analysis.sites.is_empty() {
            continue;
        }
        let resolvable = analysis.sites.iter().all(|site| {
            fmt_site_keys(name, site)
                .is_some_and(|keys| keys.iter().all(|key| graph.nodes.contains_key(key)))
        });
        if resolvable {
            out.insert(name.clone(), analysis);
        }
    }
    out
}

/// The label keys one proven site's commit is judged by: the destination (the `format!` result,
/// or the pointee behind a `write_fmt` receiver) first, then each `String` display source.
/// `None` when a source path steps through something the value graph has no key for.
fn fmt_site_keys(name: &JsName, site: &semantic_ops::fmt_kernel::Site) -> Option<Vec<NodeKey>> {
    use semantic_ops::fmt_kernel::{DisplayKind, SiteKind};
    let mut keys = Vec::new();
    keys.push(match site.kind {
        SiteKind::ToString { result } => NodeKey::Value {
            item: name.clone(),
            site: Site::Value(result),
            path: Vec::new(),
        },
        SiteKind::WriteFmt { receiver, .. } => NodeKey::Value {
            item: name.clone(),
            site: Site::Value(receiver),
            path: vec![Step::Deref],
        },
    });
    for arg in &site.args {
        if arg.kind != DisplayKind::String {
            continue;
        }
        let value_site = match arg.root {
            Root::Value(v) => Site::Value(v),
            Root::Cell(c) => Site::Cell(c),
        };
        let mut path = Vec::new();
        for proj in &arg.path {
            match proj {
                Proj::Deref => path.push(Step::Deref),
                _ => return None,
            }
        }
        keys.push(NodeKey::Value {
            item: name.clone(),
            site: value_site,
            path,
        });
    }
    Some(keys)
}

/// Drops the legality terms tying a proven consumer's values to the machinery items its
/// committed sites stop calling — the v2 spelling of v1's `proven_format_consumer_blocks`
/// exemption. Only wholly-proven callees are exempted ([`semantic_ops::fmt_kernel::Analysis::
/// elidable`]); a machinery item any unproven call or non-call reference still reaches keeps
/// its edges, so its callers keep their baseline labels.
fn filter_format_terms(
    terms: &mut Vec<Term>,
    fmt: &BTreeMap<JsName, semantic_ops::fmt_kernel::Analysis>,
    lowered: &BTreeMap<ItemId, JsName>,
) {
    let mut exempt: BTreeSet<(&JsName, &JsName)> = BTreeSet::new();
    for (consumer, analysis) in fmt {
        for item in &analysis.elidable {
            if let Some(callee) = lowered.get(item) {
                exempt.insert((consumer, callee));
            }
        }
    }
    if exempt.is_empty() {
        return;
    }
    fn item_of(key: &NodeKey) -> Option<&JsName> {
        match key {
            NodeKey::Value { item, .. } => Some(item),
            _ => None,
        }
    }
    terms.retain(|term| {
        let (a, b) = match term {
            Term::MustEqual { a, b, .. } => (a, b),
            Term::Adapter { a, b, .. } => (a, b),
            Term::UnaryInf { .. } | Term::FixedCarrier { .. } => return true,
        };
        match (item_of(a), item_of(b)) {
            (Some(x), Some(y)) => !(exempt.contains(&(x, y)) || exempt.contains(&(y, x))),
            _ => true,
        }
    });
}

/// The operations the solver may never commit this wave: the `format_args!` trio, whose contract
/// needs the call-site format kernel (see the module documentation).
fn solver_blocked(operation: StandardOperation) -> bool {
    matches!(
        operation,
        StandardOperation::FormatArgsNew
            | StandardOperation::FormatArgumentNewDisplay
            | StandardOperation::FormatArgsToString
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::valueflow::harness::*;
    use crate::jir::repr::{FuncBuilder, RTerm};
    use crate::jir::semantic::CallSemantic;

    fn artifact_cost(
        bytes: u64,
        gzip: u64,
        brotli: u64,
        allocations: u64,
        loop_allocations: u64,
        alloc_term: u64,
    ) -> ProgramCost {
        ProgramCost {
            bytes,
            helper_bytes: 0,
            gzip,
            brotli,
            allocations,
            loop_allocations,
            alloc_term,
        }
    }

    #[test]
    fn release_profiles_apply_distinct_zero_growth_carrier_policies() {
        let before = artifact_cost(1_000, 500, 400, 4, 2, 192);
        let byte_win = artifact_cost(990, 499, 399, 5, 2, 240);
        assert!(CarrierEconomics::Size.accepts(before, byte_win));
        assert!(!CarrierEconomics::Balanced.accepts(before, byte_win));

        let compressed_win = artifact_cost(1_000, 499, 399, 5, 2, 240);
        assert!(CarrierEconomics::Size.accepts(before, compressed_win));
        assert_eq!(CarrierEconomics::Size.benefit(before, compressed_win), 2);

        let straight_allocation_win = artifact_cost(1_000, 500, 400, 3, 2, 144);
        assert!(CarrierEconomics::Balanced.accepts(before, straight_allocation_win));
        assert!(!CarrierEconomics::Size.accepts(before, straight_allocation_win));
        assert!(!CarrierEconomics::Speed.accepts(before, straight_allocation_win));

        let loop_allocation_win = artifact_cost(1_000, 500, 400, 3, 1, 96);
        assert!(CarrierEconomics::Speed.accepts(before, loop_allocation_win));
        assert!(!CarrierEconomics::Speed.accepts(before, artifact_cost(999, 499, 399, 4, 3, 192),));
        assert!(!CarrierEconomics::Speed.accepts(before, artifact_cost(1_001, 500, 400, 3, 1, 96),));
    }

    #[test]
    fn carrier_economics_follow_the_active_release_profile() {
        let mut opts = crate::opts::Opts {
            profile: Some(crate::opts::Profile::ReleaseSize),
            ..crate::opts::Opts::default()
        };
        assert_eq!(CarrierEconomics::from_opts(&opts), CarrierEconomics::Size);
        opts.profile = Some(crate::opts::Profile::ReleaseBalanced);
        assert_eq!(
            CarrierEconomics::from_opts(&opts),
            CarrierEconomics::Balanced
        );
        opts.profile = Some(crate::opts::Profile::ReleaseSpeed);
        assert_eq!(CarrierEconomics::from_opts(&opts), CarrierEconomics::Speed);
    }

    /// Builds the planner over `prog` exactly as `plan_and_commit` would and hands it to `f`.
    ///
    /// The harness carries its interns beside the table (a test item has no object), so the
    /// planner is driven directly rather than through `plan_and_commit`'s interns lookup.
    fn with_planner<R>(prog: &Prog, f: impl FnOnce(&Planner<'_>, &mut Problem, &[Term]) -> R) -> R {
        let items = crate::lower::js::host::ItemIndex::new();
        let drop_glue = crate::lower::js::host::DropGlueIndex::new();
        let vtables = crate::lower::js::unsize::VtableIndex::new();
        let opts = crate::opts::Opts::default();
        let cx = ReprCx {
            opts: &opts,
            items: &items,
            drop_glue: &drop_glue,
            vtables: &vtables,
        };
        let interns_map: BTreeMap<JsName, &crate::jir::Interns> = prog
            .table
            .keys()
            .map(|name| (name.clone(), &prog.interns))
            .collect();
        let live: BTreeSet<JsName> = prog.table.keys().cloned().collect();
        let mut graph = valueflow::collect(&prog.table, &prog.funcs, &interns_map, &live);
        free_version_pins(&mut graph);
        let terms = legality::terms(&graph);
        let mut problem = Problem::build(&graph, &terms);
        let fmt = BTreeMap::new();
        let planner = Planner {
            table: &prog.table,
            funcs: &prog.funcs,
            live: &live,
            cx: &cx,
            interns: &interns_map,
            graph: &graph,
            lowered: prog
                .funcs
                .iter()
                .filter_map(|(name, func)| Some((func.as_ref().ok()?.item, name.clone())))
                .collect(),
            fmt: &fmt,
            member_bytes: cost::helper_bytes(),
            reached: reached_members(&prog.table),
        };
        f(&planner, &mut problem, &terms)
    }

    fn plan(prog: &Prog, apply: bool) -> ReprOutcome {
        with_planner(prog, |planner, problem, terms| {
            let (outcome, funcs) = planner.run(problem, terms, apply);
            if apply {
                outcome
            } else {
                assert!(funcs.is_empty());
                ReprOutcome {
                    block: outcome.block,
                    touched: BTreeSet::new(),
                    bodies: BTreeMap::new(),
                    clones: BTreeMap::new(),
                    call_retargets: BTreeMap::new(),
                }
            }
        })
    }

    /// A plan over ordinary items with no registry operation finds the candidates, keeps them
    /// baseline (nothing to gain), commits nothing, and answers identically twice.
    #[test]
    fn the_accounting_is_deterministic_and_commits_nothing_without_operations() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        prog.add(
            "caller",
            call_func(1, STRING, direct(2), CallSemantic::Ordinary, None),
        );
        let one = plan(&prog, true);
        let two = plan(&prog, true);
        assert!(one.block.candidates > 0);
        assert_eq!(one.block.committed, 0);
        assert!(one.bodies.is_empty() && one.touched.is_empty());
        assert_eq!(format!("{:?}", one.block), format!("{:?}", two.block));
    }

    /// `report` (apply = false) runs the same solve but returns no commit at all.
    #[test]
    fn report_mode_computes_the_block_and_disowns_the_commit() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        let outcome = plan(&prog, false);
        assert!(outcome.bodies.is_empty());
        assert!(outcome.touched.is_empty());
        assert!(outcome.block.candidates > 0);
    }

    /// The lane-retry rollback: `emptied` keeps the accounting, disowns the commit, and marks
    /// every kept decision as rolled back so the sidecar describes the printed program.
    #[test]
    fn emptied_disowns_the_commit_and_marks_the_kept_decisions() {
        let outcome = ReprOutcome {
            block: ReprBlock {
                families: 3,
                candidates: 2,
                legal: 2,
                cost_positive: 1,
                committed: 1,
                decisions: vec![
                    ReprDecision {
                        subject: "a$h1/v0".to_owned(),
                        family: "string".to_owned(),
                        chosen: "string-builder".to_owned(),
                        alternatives: vec!["host-string".to_owned(), "string-builder".to_owned()],
                        decline: None,
                        items: BTreeSet::new(),
                    },
                    ReprDecision {
                        subject: "b$h2/v0".to_owned(),
                        family: "carrier".to_owned(),
                        chosen: "baseline".to_owned(),
                        alternatives: vec!["baseline".to_owned()],
                        decline: Some("legality.raw-provenance".to_owned()),
                        items: BTreeSet::new(),
                    },
                ],
                adapters: 2,
                versions: 0,
                helper_bytes: 99,
                pinned_cost_positive: 0,
                exports: Vec::new(),
                ..ReprBlock::default()
            },
            touched: [JsName::new("a$h1".to_owned())].into_iter().collect(),
            bodies: BTreeMap::new(),
            clones: BTreeMap::new(),
            call_retargets: BTreeMap::new(),
        };
        let emptied = outcome.emptied();
        assert_eq!(emptied.block.committed, 0);
        assert_eq!(emptied.block.adapters, 0);
        assert_eq!(emptied.block.helper_bytes, 0);
        assert_eq!(emptied.block.candidates, 2, "the accounting survives");
        assert!(emptied.touched.is_empty() && emptied.bodies.is_empty());
        assert_eq!(emptied.block.decisions[0].chosen, "baseline");
        assert!(emptied.block.decisions[0]
            .decline
            .as_deref()
            .unwrap()
            .starts_with("version.carrier-commit-rollback"));
        // A decision that already declined keeps its own reason.
        assert_eq!(
            emptied.block.decisions[1].decline.as_deref(),
            Some("legality.raw-provenance")
        );
    }

    /// A summarized standard-operation body's own calls owe implication arcs (D4's coherence):
    /// the callee goes native only with the summarized caller's slot, and a caller with no slot
    /// node pins the callee outright.
    #[test]
    fn summarized_callers_couple_their_callees_to_their_slots() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        let mut b = FuncBuilder::new(item(1), sig(vec![STRING], UNIT));
        let p = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![p],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add_semantic(
            "op",
            b.finish(),
            BodySemantic::Standard(StandardOperation::StringPushStr),
        );
        with_planner(&prog, |planner, problem, _| {
            let pairs = planner.summarized_call_pairs(problem);
            let caller_slot = problem.resolution[&value_node("op", Site::Value(p), &[])];
            let callee_node =
                problem.resolution[&value_node("callee", Site::Value(ValueId(0)), &[])];
            assert!(
                pairs.iter().any(|pair| pair.a == callee_node
                    && pair.b == caller_slot
                    && pair.ab == INF
                    && pair.ba == 0),
                "{pairs:?}"
            );
        });
    }

    /// The degenerate half of the same coherence rule: a summarized caller with no slot node at
    /// all pins its callee outright.
    #[test]
    fn a_summarized_caller_without_slots_pins_its_callee() {
        let mut prog = Prog::new();
        prog.add("callee", identity_func(2, STRING, STRING));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        b.effect(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: Vec::new(),
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add_semantic(
            "op",
            b.finish(),
            BodySemantic::Standard(StandardOperation::StringPushStr),
        );
        with_planner(&prog, |planner, problem, _| {
            let pairs = planner.summarized_call_pairs(problem);
            assert!(pairs.is_empty(), "{pairs:?}");
            let callee_node =
                problem.resolution[&value_node("callee", Site::Value(ValueId(0)), &[])];
            assert!(problem.nodes[callee_node]
                .inf
                .contains(&crate::analysis::legality::InfReason::UnknownCall));
        });
    }

    /// Builds `fn caller() { <local vec>; push_like(&mut vec, <string>) }` with a
    /// `Standard(VecPush)` call into an Ordinary raw-view-poisoned callee: the D2 cut's shape.
    fn cut_call_prog() -> (Prog, crate::jir::repr::ValueId) {
        use crate::jir::semantic::ConstTree;
        let mut prog = Prog::new();
        let mut b = FuncBuilder::new(item(2), sig(vec![MUT_VEC, STRING], UNIT));
        let receiver = b.param(MUT_VEC, NO_ORIGIN);
        let _value = b.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        b.op(
            entry,
            OpKind::RawView {
                root: Root::Value(receiver),
                path: Vec::new(),
                mutable: true,
            },
            MUT_VEC,
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("push_like", b.finish());
        let mut c = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let v = c.op(entry, OpKind::Const(ConstTree::Unit), VEC, NO_ORIGIN);
        let r = c.op(
            entry,
            OpKind::BorrowView {
                root: Root::Value(v),
                path: Vec::new(),
                mutable: true,
            },
            MUT_VEC,
            NO_ORIGIN,
        );
        let s = c.op(entry, OpKind::Const(ConstTree::Unit), STRING, NO_ORIGIN);
        c.effect(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![r, s],
                track_caller: None,
                semantic: CallSemantic::Standard(StandardOperation::VecPush),
                host: None,
            },
            NO_ORIGIN,
        );
        c.terminate(entry, RTerm::Return(None));
        prog.add("caller", c.finish());
        (prog, r)
    }

    /// A D2 reference arc is directional only when an executable private callee version exists.
    /// Without one, the baseline callee must pin the caller instead of failing the whole commit.
    #[test]
    fn cut_standard_calls_require_an_executable_version_for_mixed_references() {
        let (prog, r) = cut_call_prog();
        with_planner(&prog, |planner, problem, _| {
            let caller_ref =
                problem.resolution[&value_node("caller", Site::Value(r), &[Step::Deref])];
            let callee_name = JsName::new("push_like".to_owned());
            let Ok(callee_func) = &prog.funcs[&callee_name] else {
                unreachable!()
            };
            let callee_ref = problem.resolution[&value_node(
                "push_like",
                Site::Value(callee_func.params[0]),
                &[Step::Deref],
            )];
            let pairs = planner.standard_call_pairs(problem, &BTreeSet::new());
            assert!(
                pairs.iter().any(|pair| pair.a == callee_ref
                    && pair.b == caller_ref
                    && pair.ab == INF
                    && pair.ba == INF
                    && pair.native_equal),
                "{pairs:?}"
            );
            let pairs = planner.standard_call_pairs(problem, &BTreeSet::from([callee_name]));
            assert!(
                pairs.iter().any(|pair| pair.a == callee_ref
                    && pair.b == caller_ref
                    && pair.ab == INF
                    && pair.ba == 0
                    && pair.native_equal),
                "{pairs:?}"
            );
        });
    }

    /// The commit's soundness net at reference slots: a cut standard call whose caller label
    /// disagrees with the callee's refuses the commit, naming both items' labeled nodes.
    #[test]
    fn a_cut_reference_slot_label_disagreement_refuses_the_commit() {
        let (prog, r) = cut_call_prog();
        let name = JsName::new("caller".to_owned());
        with_planner(&prog, |planner, problem, _| {
            let node = problem.resolution[&value_node("caller", Site::Value(r), &[Step::Deref])];
            let mut carriers: Vec<Option<Carrier>> = vec![None; problem.nodes.len()];
            carriers[node] = Some(Carrier::NativeVec);
            let func = prog.funcs[&name].as_ref().expect("the func lowered");
            let error = planner
                .commit_func(problem, &name, func, &carriers, &BTreeMap::new(), false)
                .expect_err("the disagreement refuses the commit");
            assert!(error.1.contains("disagrees"), "{}", error.1);
            assert!(error.0.contains(&node), "{:?}", error.0);
        });
    }

    /// A public-boundary pin no longer poisons the private-version solve. With no byte win the
    /// exact cost gate still keeps the original-only program.
    #[test]
    fn a_pin_is_clone_eligible_but_still_needs_an_exact_byte_win() {
        let mut prog = Prog::new();
        prog.add("named", identity_func(1, STRING, STRING));
        prog.add_unlowered("baseline_text", None, &["named"]);
        let outcome = plan(&prog, false);
        assert!(outcome.block.legal >= 1, "{:?}", outcome.block);
        assert_eq!(outcome.block.pinned_cost_positive, 0);
        assert_eq!(outcome.block.versions, 0);
        assert!(outcome
            .block
            .decisions
            .iter()
            .any(|decision| { decision.decline.as_deref() == Some("cost.region-no-shrink") }));
    }

    #[test]
    fn a_native_labeled_pinned_boundary_allocates_one_private_version() {
        let mut prog = Prog::new();
        prog.add("named", identity_func(1, STRING, STRING));
        prog.add_unlowered("baseline_text", None, &["named"]);
        with_planner(&prog, |planner, problem, _| {
            let node = problem.resolution[&value_node("named", Site::Value(ValueId(0)), &[])];
            let mut carriers = vec![None; problem.nodes.len()];
            carriers[node] = Some(Carrier::HostString);
            let names = planner
                .allocate_clone_names(problem, &carriers, &BTreeSet::new())
                .expect("one private version");
            assert_eq!(names[&JsName::new("named".to_owned())].as_str(), "named$n1");
        });
    }

    #[test]
    fn a_private_version_keeps_the_source_and_retargets_the_prepared_caller() {
        let mut prog = Prog::new();
        prog.add("named", identity_func(2, STRING, STRING));
        let mut caller = FuncBuilder::new(item(1), sig(vec![STRING], STRING));
        let argument = caller.param(STRING, NO_ORIGIN);
        let entry = FuncBuilder::ENTRY;
        let result = caller.op(
            entry,
            OpKind::Call {
                callee: direct(2),
                function: None,
                args: vec![argument],
                track_caller: None,
                semantic: CallSemantic::Ordinary,
                host: None,
            },
            STRING,
            NO_ORIGIN,
        );
        caller.terminate(entry, RTerm::Return(Some(result)));
        prog.add("caller", caller.finish());
        prog.add_unlowered("baseline_text", None, &["named"]);
        with_planner(&prog, |planner, problem, _| {
            let mut carriers = vec![None; problem.nodes.len()];
            for (index, node) in problem.nodes.iter().enumerate() {
                if node.family == Some(CandidateKind::String) && node.inf.is_empty() {
                    carriers[index] = Some(Carrier::HostString);
                }
            }
            let commit = planner
                .build_commit(problem, &carriers, &BTreeMap::new(), false)
                .expect("the private version commits");
            let clone = JsName::new("named$n1".to_owned());
            assert_eq!(commit.clones.get(&clone).map(JsName::as_str), Some("named"));
            assert!(commit.funcs.contains_key(&clone));
            assert!(!commit.funcs.contains_key(&JsName::new("named".to_owned())));
            assert_eq!(
                commit.call_retargets[&JsName::new("caller".to_owned())]
                    .get(&item(2))
                    .map(JsName::as_str),
                Some("named$n1")
            );
        });
    }

    /// The published export-representation contract: every exported candidate slot gets a row,
    /// `baseline` where nothing committed.
    #[test]
    fn exported_slots_publish_their_representation_rows() {
        let mut prog = Prog::new();
        prog.add("give", identity_func(1, STRING, STRING));
        prog.exported("give");
        let outcome = plan(&prog, false);
        assert!(
            !outcome.block.exports.is_empty(),
            "{:?}",
            outcome.block.exports
        );
        for row in &outcome.block.exports {
            assert!(row.subject.starts_with("give"), "{row:?}");
            assert_eq!(row.chosen, "baseline");
        }
    }

    /// The alias-anchor guard: a region whose ordinary members are all pure aliases is forced
    /// baseline; a region with an owned (empty-path) member is anchored and passes.
    #[test]
    fn anchor_guard_kills_alias_only_regions_and_keeps_anchored_ones() {
        let alias = NodeKey::Value {
            item: JsName::new("f".to_owned()),
            site: Site::Value(ValueId(0)),
            path: vec![Step::Deref],
        };
        let owned = NodeKey::Value {
            item: JsName::new("g".to_owned()),
            site: Site::Value(ValueId(0)),
            path: Vec::new(),
        };
        let mut problem = Problem::default();
        for key in [&alias, &owned] {
            let index = problem.nodes.len();
            problem.nodes.push(crate::plan::graph::ProblemNode {
                key: ProblemKey::Value((*key).clone()),
                family: Some(CandidateKind::String),
                mutable: false,
                fixed: None,
                inf: BTreeSet::new(),
                members: 1,
            });
            problem.resolution.insert((*key).clone(), index);
        }
        let regions = vec![BTreeSet::from([0]), BTreeSet::from([1])];
        anchor_guard(&mut problem, &regions, &BTreeMap::new());
        assert!(problem.nodes[0]
            .inf
            .contains(&crate::analysis::legality::InfReason::UnsupportedOperation));
        assert!(problem.nodes[1].inf.is_empty());
    }

    /// A `ProblemNode::fixed` carrier constrains the F3 string sub-solve: the other native
    /// string carrier is never chosen for the node.
    #[test]
    fn a_fixed_carrier_constrains_the_string_family_solve() {
        let mut prog = Prog::new();
        prog.add("f", identity_func(1, STRING, STRING));
        with_planner(&prog, |planner, _, _| {
            for fixed in [Carrier::HostString, Carrier::StringBuilder] {
                let mut problem = Problem::default();
                problem.nodes.push(crate::plan::graph::ProblemNode {
                    key: ProblemKey::Value(value_node("f", Site::Value(ValueId(0)), &[])),
                    family: Some(CandidateKind::String),
                    mutable: false,
                    fixed: Some(fixed),
                    inf: BTreeSet::new(),
                    members: 1,
                });
                problem.nodes.push(crate::plan::graph::ProblemNode {
                    key: ProblemKey::Baseline,
                    family: None,
                    mutable: false,
                    fixed: None,
                    inf: BTreeSet::new(),
                    members: 1,
                });
                problem.baseline = 1;
                // Baseline costs 10 against a free native label, so F1 goes native; F3 must
                // then take exactly the fixed carrier.
                let unaries = (vec![10, 0], vec![0, INF]);
                let (carriers, f1, _, infeasible) = planner.solve_families(
                    &problem,
                    &BTreeMap::new(),
                    &[],
                    &unaries,
                    &BTreeSet::new(),
                );
                assert!(infeasible.is_empty());
                assert!(f1[0]);
                assert_eq!(carriers[0], Some(fixed));
            }
        });
    }

    /// The rollback accounting: a region the transaction rolled back stays cost-positive but
    /// uncommitted, and its decision carries the rollback decline.
    #[test]
    fn a_rolled_back_region_is_cost_positive_but_not_committed() {
        let mut prog = Prog::new();
        prog.add("f", identity_func(1, STRING, STRING));
        with_planner(&prog, |planner, problem, terms| {
            let regions = regions(problem, terms);
            assert!(!regions.is_empty());
            let node = *regions[0].iter().next().unwrap();
            let count = problem.nodes.len();
            let carriers: Vec<Option<Carrier>> = vec![None; count];
            let unaries = (vec![0u64; count], vec![0u64; count]);
            let rollbacks: BTreeMap<usize, String> = [(
                node,
                "version.carrier-commit-rollback RCGJS_REPR_FAULT".to_owned(),
            )]
            .into_iter()
            .collect();
            let block = planner.account(
                problem,
                &regions,
                &carriers,
                &[],
                &unaries,
                &BTreeMap::new(),
                &rollbacks,
                &BTreeSet::new(),
                &BTreeSet::from([node]),
                &Commit::default(),
                1,
            );
            assert_eq!(block.committed, 0);
            assert_eq!(block.cost_positive, 1);
            let decision = block
                .decisions
                .iter()
                .find(|decision| {
                    decision.decline.as_deref().is_some_and(|decline| {
                        decline.starts_with("version.carrier-commit-rollback")
                    })
                })
                .expect("the rolled-back region's decision");
            assert_eq!(decision.chosen, "baseline");
        });
    }

    /// The rollback trigger: a boxed cell dropped across a carrier cut cannot be rewritten in
    /// place, so `commit_func` refuses and the region rolls back.
    #[test]
    fn a_boxed_cell_dropped_across_a_cut_refuses_the_commit() {
        let mut prog = Prog::new();
        prog.add("glue", identity_func(9, MUT_STRING, UNIT));
        let mut b = FuncBuilder::new(item(1), sig(Vec::new(), UNIT));
        let entry = FuncBuilder::ENTRY;
        let cell = b.cell(STRING, true, NO_ORIGIN);
        b.effect(entry, OpKind::CellAlloc(cell), NO_ORIGIN);
        b.effect(
            entry,
            OpKind::Drop {
                root: Root::Cell(cell),
                path: Vec::new(),
                glue: DropGlue::Direct {
                    item: item(9),
                    link_name: crate::jir::interns::StrId(0),
                },
            },
            NO_ORIGIN,
        );
        b.terminate(entry, RTerm::Return(None));
        prog.add("f", b.finish());
        let name = JsName::new("f".to_owned());
        with_planner(&prog, |planner, problem, _| {
            let node = problem.resolution[&value_node("f", Site::Cell(cell), &[])];
            let mut carriers: Vec<Option<Carrier>> = vec![None; problem.nodes.len()];
            carriers[node] = Some(Carrier::StringBuilder);
            let func = prog.funcs[&name].as_ref().expect("the func lowered");
            let error = planner
                .commit_func(problem, &name, func, &carriers, &BTreeMap::new(), false)
                .expect_err("the boxed cell refuses the cut");
            assert!(error.1.contains("boxed cell"), "{}", error.1);
        });
    }
}
