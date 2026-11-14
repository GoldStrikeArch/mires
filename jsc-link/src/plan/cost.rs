//! The representation planner's cost model, in minified-byte equivalents (Phase 4, D9).
//!
//! Every finite term the min-cut solver weighs comes from a compact lowering rather than a source
//! heuristic. The min-cut is only a proposal: `plan/representation.rs::program_cost` validates
//! each proposed label vector by lowering the complete program, recomputing reachability and
//! deduplication, renaming final items, and charging unique helpers and final JavaScript literal
//! sites. Only that exact whole-program result may authorize a commit.
//!
//! * **per-item bytes** ([`measure_item`]): the item lowered at a label vector by the real phase-C
//!   pipeline on a clone, printed compact through [`crate::minify::shorten_locals_for_measurement`]
//!   — the same `bytes_min` entry point the lane sidecar reports, never the raw print. Cached in
//!   [`Cost`] by the [`LabelVector::digest`] of the item's labels, so a solver round that revisits
//!   a labeling never lowers it twice.
//! * **helper bytes** ([`helper_bytes`]): a `Helper(member)` facility node costs its
//!   `runtime/shim.js` member's source bytes once program-wide. Adapter expressions never include
//!   helper source bytes, so several adapters cannot multiply-charge one facility.
//! * **allocation term** ([`Cost::alloc_term`]): `w_alloc * 4^min(depth, 3)` per fresh literal in
//!   the final lowered JavaScript AST, reported by [`crate::analysis::js_alloc`]. `w_alloc` comes
//!   from the link-only `js-opt-weight-alloc` option and defaults to 24.
//! * **adapter bytes** ([`measure_expr`]): the printed compact bytes of a `Convert`'s spelling at
//!   an edge crossing.
//! * **version fixed cost** ([`measure_decl`]): the measured bytes of a `$n` version clone.
//!
//! The solver runs at most three solve-measure rounds. A failed transaction forces its complete
//! region baseline and starts another solve, so surviving neighbors are measured again without
//! relying on the rolled-back labeling. Exact program costs are cached by the complete label
//! vector.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::analysis::js_alloc::JsAllocSites;
use crate::jir::decline::Decline;
use crate::jir::hash::Hash128;
use crate::jir::repr::{Carrier, Func, ValueId};
use crate::jsast;
use crate::lower::{LowerInput, LoweredItem};
use crate::opt::OptStats;
use crate::opts::{OptimizerMode, Opts};

/// The solver's infinity: an illegal labeling. Matches `plan/mincut.rs`'s arc capacity ceiling,
/// far enough from `u64::MAX` that summing terms cannot wrap.
pub const INF: u64 = u64::MAX / 4;

/// The consumer-side runtime, whose members price the `Helper` facility nodes.
pub(crate) const SHIM: &str = include_str!("../../../runtime/shim.js");

pub fn runtime_source_sha256() -> String {
    rcgjs_object::artifact::sha256(SHIM.as_bytes())
}

/// The carrier chosen for each value of ONE item, sorted by value id: the unit the per-item byte
/// cache is keyed by.
///
/// Only the values whose carrier differs from [`Carrier::Baseline`] need naming — an absent value
/// is baseline — so the all-baseline vector is empty and every item starts on one shared cache
/// entry per item… except that the cache is per label vector, not per item, so the caller keys
/// items apart by seeding the vector with the item (see [`LabelVector::for_item`]).
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Per-item probes retain this cache; whole-program vectors are cached by the
                    // representation planner because they include reachability outside one item.
pub struct LabelVector {
    /// The item the labels are of, as its emitted name: two items with identical labels must not
    /// share a cache entry, their bodies differ.
    item: String,
    labels: Vec<(ValueId, Carrier)>,
}

#[allow(dead_code)] // See `LabelVector`.
impl LabelVector {
    /// The vector labeling `item`'s values, unnamed values baseline. Sorted; a value named twice
    /// is a caller bug and panics here rather than mis-keying the cache.
    pub fn for_item(item: &str, mut labels: Vec<(ValueId, Carrier)>) -> LabelVector {
        labels.sort_unstable_by_key(|(value, _)| *value);
        labels.retain(|(_, carrier)| *carrier != Carrier::Baseline);
        assert!(
            labels.windows(2).all(|pair| pair[0].0 != pair[1].0),
            "a label vector names a value twice"
        );
        LabelVector {
            item: item.to_owned(),
            labels,
        }
    }

    /// The stable digest the byte cache is keyed by: a canonical byte spelling through the
    /// object format's seedless [`Hash128`], never `DefaultHasher` (determinism rule).
    pub fn digest(&self) -> u64 {
        let mut bytes = Vec::with_capacity(self.item.len() + 1 + self.labels.len() * 5);
        bytes.extend_from_slice(self.item.as_bytes());
        // Item names never contain NUL, so the terminator keeps (item, labels) unambiguous.
        bytes.push(0);
        for (value, carrier) in &self.labels {
            bytes.extend_from_slice(&value.0.to_le_bytes());
            bytes.push(*carrier as u8);
        }
        Hash128::of(&bytes).fold()
    }
}

/// A cached per-item byte figure: what round it came from decides what may replace it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // See `LabelVector`.
enum Entry {
    /// Round 1's guess. Replaced by the first measurement and never installed over one.
    Estimate(u64),
    /// The real pipeline's answer, [`INF`] for a labeling the pipeline declined.
    Measured(u64),
}

/// The byte cache and the option-fed weights: one per solve, owned by the planner.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // See `LabelVector`.
pub struct Cost {
    weight_alloc: u64,
    entries: BTreeMap<u64, Entry>,
}

/// One function lowered through the measurement pipeline, with its allocation-weight term.
pub struct MeasuredItem {
    pub lowered: LoweredItem,
    pub alloc_term: u64,
}

#[allow(dead_code)] // See `LabelVector`.
impl Cost {
    pub fn new(opts: &Opts) -> Cost {
        Cost {
            weight_alloc: opts.weight_alloc,
            entries: BTreeMap::new(),
        }
    }

    /// Seeds round 1's estimate for a labeling. A measurement already made wins; a second
    /// estimate for the same labeling is ignored rather than averaged — the first guess is as
    /// good as the second and stability is worth more.
    pub fn estimate(&mut self, labels: &LabelVector, bytes: u64) {
        self.entries
            .entry(labels.digest())
            .or_insert(Entry::Estimate(bytes));
    }

    /// The cached figure for a labeling, estimate or measured, if any.
    pub fn get(&self, labels: &LabelVector) -> Option<u64> {
        self.entries
            .get(&labels.digest())
            .map(|entry| match *entry {
                Entry::Estimate(bytes) | Entry::Measured(bytes) => bytes,
            })
    }

    /// The measured bytes of an item at a labeling, lowering it at most once.
    ///
    /// A cache hit on a measurement returns it without calling `measure`; anything else —
    /// vacant, or an estimate — runs `measure` and installs the answer over whatever was there.
    /// `measure` answering `None` is a declined lowering, cached as [`INF`] so the solver
    /// labels around it without re-lowering it every round.
    pub fn of(&mut self, labels: &LabelVector, measure: impl FnOnce() -> Option<u64>) -> u64 {
        let key = labels.digest();
        if let Some(Entry::Measured(bytes)) = self.entries.get(&key) {
            return *bytes;
        }
        let bytes = measure().unwrap_or(INF);
        self.entries.insert(key, Entry::Measured(bytes));
        bytes
    }

    /// The allocation term of one final JavaScript declaration.
    pub fn alloc_term(&self, sites: &JsAllocSites) -> u64 {
        sites.weighted_term(self.weight_alloc)
    }
}

/// The measured minified bytes of one item at the labeling already applied to `func`, through
/// the real phase-C pipeline on a clone: optimize, lower, shorten, print compact.
///
/// `None` is a decline anywhere along the pipeline — the labeling has no lowering, which the
/// caller prices as [`INF`].
///
/// The pipeline is forced to [`OptimizerMode::On`]: under `js-opt=report`
/// `crate::opt::optimize` measures on a clone and drops the rewrite, and a cost taken from the
/// unoptimized print would make `report`'s solve disagree with `on`'s — `report` must say what
/// `on` would do.
pub fn measure_item(input: &LowerInput<'_>, func: &Func) -> Option<u64> {
    crate::profile::bump(crate::profile::Counter::LowerReprProbe);
    let measured = lower_item(input, func).ok()?;
    Some(measure_decl(&measured.lowered.decl))
}

/// Lowers one function with optimizer mode forced on, returning the exact declaration and the
/// allocation term of the final JIR-R body. This is shared by per-item probes and the final
/// post-commit whole-program measurement.
pub fn lower_item(input: &LowerInput<'_>, func: &Func) -> Result<MeasuredItem, Decline> {
    crate::profile::bump(crate::profile::Counter::LowerItem);
    let mut func = func.clone();
    let mut opts = input.opts.clone();
    opts.optimizer = OptimizerMode::On;
    let input = LowerInput {
        opts: &opts,
        ..*input
    };
    let mut stats = OptStats::default();
    crate::opt::optimize(&mut func, input.table, input.interns, &opts, &mut stats)?;
    let lowered = crate::lower::lower_func(&input, &func)?;
    let alloc_term = Cost::new(&opts).alloc_term(&JsAllocSites::of(&lowered.decl));
    Ok(MeasuredItem {
        lowered,
        alloc_term,
    })
}

/// The bytes one declaration takes printed compact with its locals shortened — the sidecar's
/// `bytes_min` measurement, on a clone. Prices a version clone (D9's fixed cost) as well as a
/// solver probe's lowering.
pub fn measure_decl(decl: &jsast::Stmt) -> u64 {
    compact_decl(decl).len() as u64
}

/// Prices an inlining probe with the program's allocated item names.
pub fn measure_inline_item(
    input: &LowerInput<'_>,
    func: &Func,
    renames: &BTreeMap<String, String>,
) -> Option<(u64, BTreeSet<crate::item::JsName>)> {
    crate::profile::bump(crate::profile::Counter::LowerInline);
    let measured = lower_item(input, func).ok()?;
    let compact = compact_decl(&measured.lowered.decl);
    Some((
        crate::minify::apply_renames(&compact, renames).len() as u64,
        measured.lowered.refs,
    ))
}

fn compact_decl(decl: &jsast::Stmt) -> String {
    let mut measured = decl.clone();
    crate::minify::shorten_locals_for_measurement(&mut measured);
    jsast::program_to_string_styled_for_target(
        std::slice::from_ref(&measured),
        jsast::Style::Compact,
        crate::opts::get().ecma_target,
    )
}

/// The bytes one adapter expression takes printed compact: what a `Convert` costs at each edge
/// crossing it is spelled at.
pub fn measure_expr(expr: &jsast::Expr) -> u64 {
    let stmt = jsast::Stmt::ExprStmt(expr.clone());
    jsast::program_to_string_styled_for_target(
        std::slice::from_ref(&stmt),
        jsast::Style::Compact,
        crate::opts::get().ecma_target,
    )
    .len() as u64
}

/// Every `__rt` member's source bytes, by name: the price of a `Helper(member)` facility node.
///
/// Parsed from the bundled `runtime/shim.js` exactly as `scripts/minimal-shim.mjs` slices
/// members, so the byte a helper is priced at is the byte `minimal-shim` ships.
pub fn helper_bytes() -> BTreeMap<String, u64> {
    shim()
        .members
        .iter()
        .map(|(name, source)| (name.clone(), source.len() as u64))
        .collect()
}

/// The transitive runtime members reached by `direct`, using the same member statements and
/// reference closure as `scripts/minimal-shim.mjs`.
pub fn helper_closure(direct: &BTreeSet<String>) -> BTreeSet<String> {
    shim().closure(direct)
}

/// Every runtime helper directly mentioned by a compact program.
pub fn runtime_members(program: &str) -> BTreeSet<String> {
    referenced_members(program)
}

/// Deterministic source for the transitive runtime-helper closure of `direct`.
pub fn helper_program(direct: &BTreeSet<String>) -> String {
    let shim = shim();
    let mut program = String::new();
    for member in shim.closure(direct) {
        if let Some(source) = shim.members.get(&member) {
            program.push_str(source);
            program.push('\n');
        }
    }
    program
}

/// The bundled `runtime/shim.js`, parsed once per process.
fn shim() -> &'static ShimIndex {
    static SHIM_INDEX: OnceLock<ShimIndex> = OnceLock::new();
    SHIM_INDEX.get_or_init(|| ShimIndex::of(SHIM))
}

/// One shim source's member statements, and each member's direct dependencies under
/// `minimal-shim.mjs`'s closure rule. Neither depends on which members a program reaches, so a
/// closure is a walk over this index rather than a re-parse.
struct ShimIndex {
    /// Every member's statement, by name ([`shim_members`]).
    members: BTreeMap<String, String>,
    /// The members each member's statement reaches directly: the runtime members it references,
    /// and the owner of every lexical helper binding it mentions.
    dependencies: BTreeMap<String, BTreeSet<String>>,
}

impl ShimIndex {
    fn of(source: &str) -> ShimIndex {
        let members = shim_members(source);
        // Match minimal-shim's source groups: lexical helper declarations travel
        // with the runtime member preceding them, even when another member calls them.
        let mut bindings = BTreeMap::new();
        for (name, source) in &members {
            for line in source.lines() {
                let Some(declaration) = ["const ", "let ", "var ", "function "]
                    .into_iter()
                    .find_map(|prefix| line.strip_prefix(prefix))
                else {
                    continue;
                };
                let binding = declaration
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                    .next()
                    .and_then(ident);
                if let Some(binding) = binding {
                    bindings.insert(binding.to_owned(), name.clone());
                }
            }
        }
        let dependencies = members
            .iter()
            .map(|(name, statement)| {
                let mut dependencies = referenced_members(statement);
                for token in
                    statement.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                {
                    if let Some(owner) = bindings.get(token) {
                        dependencies.insert(owner.clone());
                    }
                }
                (name.clone(), dependencies)
            })
            .collect();
        ShimIndex {
            members,
            dependencies,
        }
    }

    /// `direct` and every member it transitively depends on. A name that is not a member is
    /// kept and has no dependencies.
    fn closure(&self, direct: &BTreeSet<String>) -> BTreeSet<String> {
        crate::profile::bump(crate::profile::Counter::ShimClosure);
        let mut needed = direct.clone();
        let mut queue: Vec<&str> = direct.iter().map(String::as_str).collect();
        while let Some(name) = queue.pop() {
            let Some(dependencies) = self.dependencies.get(name) else {
                continue;
            };
            for dependency in dependencies {
                if needed.insert(dependency.clone()) {
                    queue.push(dependency);
                }
            }
        }
        needed
    }
}

/// Every member statement in any shim source. Split out so the parser and closure are testable.
///
/// The shim's rigid shape (its own documented contract, relied on by `minimal-shim.mjs`): one
/// opening `globalThis.__rt = { name: body, … };` literal, then one top-level
/// `globalThis.__rt.name = …` assignment per member, each ending immediately before the next
/// member's leading comments.
fn shim_members(source: &str) -> BTreeMap<String, String> {
    crate::profile::bump(crate::profile::Counter::ShimMembers);
    let lines: Vec<&str> = source.lines().collect();
    let mut members = BTreeMap::new();

    let open = lines
        .iter()
        .position(|line| line.starts_with("globalThis.__rt = {"))
        .expect("the shim opens with `globalThis.__rt = {`");
    let close = lines[open..]
        .iter()
        .position(|line| *line == "};")
        .map(|offset| open + offset)
        .expect("the shim's opening object literal is closed");

    // The literal's entries, spelled as the standalone assignment minimal-shim publishes them as.
    let mut entry: Option<(String, Vec<&str>)> = None;
    let flush = |entry: &mut Option<(String, Vec<&str>)>,
                 members: &mut BTreeMap<String, String>| {
        if let Some((name, body)) = entry.take() {
            let body = body.join("\n");
            let body = body.trim_end().trim_end_matches(',');
            let statement = format!("globalThis.__rt.{name} = {body};");
            members.insert(name, statement);
        }
    };
    for line in &lines[open + 1..close] {
        if let Some((name, rest)) = literal_entry(line) {
            flush(&mut entry, &mut members);
            entry = Some((name, vec![rest]));
        } else if let Some((_, body)) = entry.as_mut() {
            if !line.trim_start().starts_with("//") {
                body.push(line);
            }
        }
    }
    flush(&mut entry, &mut members);

    // The later members: assignment start to the last non-blank, non-comment line before the next.
    let starts: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .skip(close + 1)
        .filter_map(|(index, line)| Some((index, assignment_name(line)?)))
        .collect();
    for (position, (from, name)) in starts.iter().enumerate() {
        let mut to = starts
            .get(position + 1)
            .map_or(lines.len(), |(next, _)| *next)
            - 1;
        while to > *from && (lines[to].trim().is_empty() || lines[to].starts_with("//")) {
            to -= 1;
        }
        members.insert(name.clone(), lines[*from..=to].join("\n"));
    }
    members
}

/// The closure of `direct` over any shim source, parsed here. The bundled shim's closure is
/// [`helper_closure`], served from its parsed-once index.
#[cfg(test)]
fn shim_helper_closure(source: &str, direct: &BTreeSet<String>) -> BTreeSet<String> {
    ShimIndex::of(source).closure(direct)
}

/// Every runtime member reference in one statement. The boundary check mirrors
/// `minimal-shim.mjs`: a property embedded in a longer identifier or property chain is ignored.
fn referenced_members(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut offset = 0;
    while let Some(relative) = source[offset..].find("__rt.") {
        let at = offset + relative;
        let global = source[..at].ends_with("globalThis.");
        let boundary = if global { at.saturating_sub(11) } else { at };
        let before = source[..boundary].chars().next_back();
        let valid_boundary =
            before.is_none_or(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.')));
        let name_at = at + "__rt.".len();
        let rest = &source[name_at..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '$')))
            .unwrap_or(rest.len());
        if valid_boundary && end > 0 {
            found.insert(rest[..end].to_owned());
        }
        offset = name_at + end.max(1);
    }
    found
}

/// `  name: rest` at the top of one opening-literal entry, or `None`.
fn literal_entry(line: &str) -> Option<(String, &str)> {
    let rest = line.strip_prefix("  ")?;
    if rest.starts_with(' ') {
        return None;
    }
    let colon = rest.find(':')?;
    let name = &rest[..colon];
    ident(name)?;
    Some((name.to_owned(), rest[colon + 1..].trim_start()))
}

/// `globalThis.__rt.name =` at the top of one member assignment, or `None`.
fn assignment_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("globalThis.__rt.")?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(rest.len());
    let name = ident(&rest[..end])?;
    rest[end..]
        .trim_start()
        .starts_with('=')
        .then(|| name.to_owned())
}

/// `name` when it is a JavaScript identifier, `None` otherwise.
fn ident(name: &str) -> Option<&str> {
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        .then_some(())
        .filter(|()| chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .map(|()| name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lexical_helper_dependencies_retain_the_owning_member() {
        let source = "globalThis.__rt = {\n};\n\
globalThis.__rt.inner = value => value;\n\
globalThis.__rt.unused = () => 0;\n\
globalThis.__rt.owner = () => 0;\n\
const localHelper = value => globalThis.__rt.inner(value);\n\
globalThis.__rt.caller = () => localHelper(7);\n";
        assert_eq!(
            super::shim_helper_closure(
                source,
                &std::collections::BTreeSet::from(["caller".to_owned()])
            ),
            ["caller", "inner", "owner"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
        );
    }

    use super::*;

    fn labels(item: &str, pairs: &[(u32, Carrier)]) -> LabelVector {
        LabelVector::for_item(
            item,
            pairs
                .iter()
                .map(|(value, carrier)| (ValueId(*value), *carrier))
                .collect(),
        )
    }

    /// The digest is a pure function of (item, labels): order-insensitive, baseline-elision
    /// canonical, and different for a different carrier, value or item.
    #[test]
    fn the_digest_keys_by_item_and_labels() {
        let one = labels("f$h1", &[(1, Carrier::HostString), (4, Carrier::NativeVec)]);
        let reordered = labels("f$h1", &[(4, Carrier::NativeVec), (1, Carrier::HostString)]);
        assert_eq!(one.digest(), reordered.digest());

        // A value labeled baseline is the same vector as one not naming it at all.
        let elided = labels(
            "f$h1",
            &[
                (1, Carrier::HostString),
                (2, Carrier::Baseline),
                (4, Carrier::NativeVec),
            ],
        );
        assert_eq!(one.digest(), elided.digest());

        let other_carrier = labels(
            "f$h1",
            &[(1, Carrier::StringBuilder), (4, Carrier::NativeVec)],
        );
        let other_value = labels("f$h1", &[(2, Carrier::HostString), (4, Carrier::NativeVec)]);
        let other_item = labels("g$h2", &[(1, Carrier::HostString), (4, Carrier::NativeVec)]);
        assert_ne!(one.digest(), other_carrier.digest());
        assert_ne!(one.digest(), other_value.digest());
        assert_ne!(one.digest(), other_item.digest());
    }

    /// A cache hit on a measurement never re-lowers; a different labeling does.
    #[test]
    fn a_measured_labeling_is_never_lowered_twice() {
        let mut cost = Cost::default();
        let one = labels("f$h1", &[(1, Carrier::HostString)]);
        let two = labels("f$h1", &[(1, Carrier::StringBuilder)]);
        let measures = std::cell::Cell::new(0);
        let of = |cost: &mut Cost, vector: &LabelVector, bytes| {
            cost.of(vector, || {
                measures.set(measures.get() + 1);
                Some(bytes)
            })
        };
        assert_eq!(of(&mut cost, &one, 120), 120);
        assert_eq!(of(&mut cost, &one, 999), 120, "the cached measurement wins");
        assert_eq!(measures.get(), 1, "the hit must not re-lower");
        assert_eq!(of(&mut cost, &two, 90), 90);
        assert_eq!(measures.get(), 2);
    }

    /// Round 2's measurement replaces round 1's estimate, and no later estimate replaces a
    /// measurement: the model only ever moves toward measured bytes.
    #[test]
    fn a_measurement_replaces_an_estimate_and_is_never_replaced_by_one() {
        let mut cost = Cost::default();
        let vector = labels("f$h1", &[(3, Carrier::NativeVec)]);
        cost.estimate(&vector, 200);
        assert_eq!(cost.get(&vector), Some(200));
        cost.estimate(&vector, 300);
        assert_eq!(cost.get(&vector), Some(200), "the first estimate stands");
        assert_eq!(
            cost.of(&vector, || Some(180)),
            180,
            "an estimate never satisfies `of`"
        );
        cost.estimate(&vector, 500);
        assert_eq!(
            cost.get(&vector),
            Some(180),
            "measured is final against estimates"
        );
        assert_eq!(cost.of(&vector, || Some(999)), 180);
    }

    /// A declined lowering is priced INF and cached like any measurement.
    #[test]
    fn a_declined_lowering_costs_inf_once() {
        let mut cost = Cost::default();
        let vector = labels("f$h1", &[(1, Carrier::Opaque)]);
        let mut measures = 0;
        assert_eq!(
            cost.of(&vector, || {
                measures += 1;
                None
            }),
            INF
        );
        assert_eq!(cost.of(&vector, || panic!("cached")), INF);
        assert_eq!(measures, 1);
    }

    /// Final JavaScript literals, including ones introduced by lowering, feed the allocation
    /// term.
    #[test]
    fn the_alloc_term_reads_the_final_js_ast() {
        let decl = jsast::Stmt::FunctionDecl {
            name: "f".to_owned(),
            params: Vec::new(),
            body: vec![
                jsast::Stmt::ExprStmt(jsast::Expr::Object(Vec::new())),
                jsast::Stmt::While(
                    jsast::Expr::Bool(true),
                    vec![jsast::Stmt::ExprStmt(jsast::Expr::Array(Vec::new()))],
                ),
            ],
        };
        let sites = JsAllocSites::of(&decl);
        let cost = Cost {
            weight_alloc: 24,
            entries: BTreeMap::new(),
        };
        assert_eq!(cost.alloc_term(&sites), 24 * (1 + 4));
        assert_eq!(cost.alloc_term(&JsAllocSites::default()), 0);
    }

    /// `measure_decl` measures the shortened compact print, not the lowering's own local names.
    #[test]
    fn a_declaration_is_measured_with_its_locals_shortened() {
        let decl = jsast::function(
            "f$h1".to_owned(),
            vec!["a_very_long_parameter".to_owned()],
            vec![jsast::Stmt::Return(Some(jsast::Expr::Ident(
                "a_very_long_parameter".to_owned(),
            )))],
        );
        let raw =
            jsast::program_to_string_styled(std::slice::from_ref(&decl), jsast::Style::Compact)
                .len() as u64;
        let measured = measure_decl(&decl);
        assert!(measured < raw, "{measured} vs {raw}");
        // Measured on a clone: the declaration itself keeps its names.
        assert!(
            jsast::program_to_string(std::slice::from_ref(&decl)).contains("a_very_long_parameter")
        );
    }

    #[test]
    fn an_adapter_expression_is_priced_by_its_compact_print() {
        let expr = jsast::call(
            jsast::Expr::Ident("__rt.bytes_str".to_owned()),
            vec![
                jsast::Expr::Ident("buf".to_owned()),
                jsast::num(0),
                jsast::Expr::Ident("len".to_owned()),
            ],
        );
        // `__rt.bytes_str(buf,0,len);` plus the program's trailing newline.
        assert_eq!(measure_expr(&expr), 27);
    }

    /// The shim parser prices every member the runtime installs, the opening literal's entries
    /// spelled exactly as `minimal-shim.mjs` publishes them.
    #[test]
    fn the_shim_members_are_priced() {
        let members = helper_bytes();
        assert_eq!(
            members.get("js_log_i32").copied(),
            Some("globalThis.__rt.js_log_i32 = (x) => console.log(x);".len() as u64)
        );
        for member in ["js_abort", "str_bytes", "bytes_str", "native_f64_to_string"] {
            assert!(
                members.get(member).is_some_and(|bytes| *bytes > 0),
                "missing or empty member {member}"
            );
        }
        // Deterministic: two parses agree exactly.
        assert_eq!(members, helper_bytes());
    }

    #[test]
    fn helper_costs_follow_transitive_runtime_references() {
        let source = r#"globalThis.__rt = {
  leaf: () => 1,
  middle: () => __rt.leaf(),
};

globalThis.__rt.root = () => globalThis.__rt.middle();
"#;
        let direct = BTreeSet::from(["root".to_owned()]);
        assert_eq!(
            shim_helper_closure(source, &direct),
            BTreeSet::from(["leaf".to_owned(), "middle".to_owned(), "root".to_owned(),])
        );
        assert_eq!(
            referenced_members("x__rt.nope(); obj.__rt.nope(); __rt.yes();"),
            BTreeSet::from(["yes".to_owned()])
        );
    }

    /// The closure as it was computed before the shim was indexed: a fresh parse, and the
    /// dependencies of each member re-derived as the walk reaches it.
    fn uncached_closure(source: &str, direct: &BTreeSet<String>) -> BTreeSet<String> {
        let members = shim_members(source);
        let mut bindings = BTreeMap::new();
        for (name, source) in &members {
            for line in source.lines() {
                let Some(declaration) = ["const ", "let ", "var ", "function "]
                    .into_iter()
                    .find_map(|prefix| line.strip_prefix(prefix))
                else {
                    continue;
                };
                let binding = declaration
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                    .next()
                    .and_then(ident);
                if let Some(binding) = binding {
                    bindings.insert(binding.to_owned(), name.clone());
                }
            }
        }
        let mut needed = direct.clone();
        let mut queue: Vec<String> = direct.iter().cloned().collect();
        while let Some(name) = queue.pop() {
            let Some(statement) = members.get(&name) else {
                continue;
            };
            let mut dependencies = referenced_members(statement);
            for token in
                statement.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
            {
                if let Some(owner) = bindings.get(token) {
                    dependencies.insert(owner.clone());
                }
            }
            for dependency in dependencies {
                if needed.insert(dependency.clone()) {
                    queue.push(dependency);
                }
            }
        }
        needed
    }

    /// `helper_program` before the shim was indexed.
    fn uncached_helper_program(direct: &BTreeSet<String>) -> String {
        let members = shim_members(SHIM);
        let closure = uncached_closure(SHIM, direct);
        let mut program = String::new();
        for member in closure {
            if let Some(source) = members.get(&member) {
                program.push_str(source);
                program.push('\n');
            }
        }
        program
    }

    /// The parsed-once shim answers exactly what a fresh parse per call did: for no member,
    /// every member alone, members `minimal-shim.mjs` would be handed by real programs, names
    /// the shim does not define, and every member at once.
    #[test]
    fn the_indexed_shim_matches_a_fresh_parse() {
        let members = shim_members(SHIM);
        let declares = |line: &str| {
            ["const ", "let ", "function "]
                .iter()
                .any(|p| line.starts_with(p))
        };
        assert!(
            members
                .values()
                .flat_map(|statement| statement.lines())
                .any(declares),
            "the shim declares lexical helpers, so the binding rule is exercised"
        );
        let set = |names: &[&str]| -> BTreeSet<String> {
            names.iter().map(|name| (*name).to_owned()).collect()
        };
        let mut directs = vec![
            BTreeSet::new(),
            set(&["js_log_i32", "js_abort"]),
            set(&["str_bytes", "bytes_str", "native_f64_to_string"]),
            set(&["not_a_member", "bytes_str"]),
            set(&["not_a_member"]),
            members.keys().cloned().collect(),
        ];
        directs.extend(members.keys().map(|name| set(&[name.as_str()])));
        let mut reaches_further = false;
        for direct in &directs {
            let closure = helper_closure(direct);
            assert_eq!(closure, uncached_closure(SHIM, direct), "{direct:?}");
            assert_eq!(
                helper_program(direct),
                uncached_helper_program(direct),
                "{direct:?}"
            );
            reaches_further |= closure.len() > direct.len();
        }
        assert!(reaches_further, "some member depends on another");
        assert_eq!(
            helper_bytes(),
            members
                .iter()
                .map(|(name, source)| (name.clone(), source.len() as u64))
                .collect::<BTreeMap<_, _>>()
        );
    }
}
