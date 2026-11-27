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
use crate::jir::repr::Func;
use crate::jsast;
use crate::lower::{LowerInput, LoweredItem};
use crate::opt::OptStats;
use crate::opts::{OptimizerMode, Opts};

/// The consumer-side runtime, whose members price helper closures.
pub(crate) const SHIM: &str = include_str!("../../../runtime/shim.js");

pub fn runtime_source_sha256() -> String {
    rcgjs_object::artifact::sha256(SHIM.as_bytes())
}

/// Option-fed weights for allocation cost pricing.
#[derive(Clone, Debug, Default)]
pub struct Cost {
    weight_alloc: u64,
}

/// One function lowered through the measurement pipeline, with its allocation-weight term.
pub struct MeasuredItem {
    pub lowered: LoweredItem,
    pub alloc_term: u64,
}

impl Cost {
    pub fn new(opts: &Opts) -> Cost {
        Cost {
            weight_alloc: opts.weight_alloc,
        }
    }

    /// The allocation term of one final JavaScript declaration.
    pub fn alloc_term(&self, sites: &JsAllocSites) -> u64 {
        sites.weighted_term(self.weight_alloc)
    }
}

/// Lowers one function with optimizer mode forced on, returning the exact declaration and the
/// allocation term of the final JIR-R body.
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
    crate::opt::optimize_for_probe(&mut func, input.table, input.interns, &opts, &mut stats)?;
    let lowered = crate::lower::lower_func(&input, &func)?;
    let alloc_term = Cost::new(&opts).alloc_term(&JsAllocSites::of(&lowered.decl));
    Ok(MeasuredItem {
        lowered,
        alloc_term,
    })
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
pub(crate) fn shim() -> &'static ShimIndex {
    static SHIM_INDEX: OnceLock<ShimIndex> = OnceLock::new();
    SHIM_INDEX.get_or_init(|| ShimIndex::of(SHIM))
}

/// One shim source's member statements, and each member's direct dependencies under
/// `minimal-shim.mjs`'s closure rule. Neither depends on which members a program reaches, so a
/// closure is a walk over this index rather than a re-parse.
pub(crate) struct ShimIndex {
    /// Member names in original `runtime/shim.js` declaration order.
    pub(crate) order: Vec<String>,
    /// Every member's statement, by name ([`shim_members_ordered`]).
    pub(crate) members: BTreeMap<String, String>,
    /// The members each member's statement reaches directly: the runtime members it references,
    /// and the owner of every lexical helper binding it mentions.
    dependencies: BTreeMap<String, BTreeSet<String>>,
}

impl ShimIndex {
    pub(crate) fn of(source: &str) -> ShimIndex {
        let (order, members) = shim_members_ordered(source);
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
            order,
            members,
            dependencies,
        }
    }

    /// `direct` and every member it transitively depends on. A name that is not a member is
    /// kept and has no dependencies.
    pub(crate) fn closure(&self, direct: &BTreeSet<String>) -> BTreeSet<String> {
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

/// Every member statement in any shim source, alongside original declaration order.
/// Split out so the parser and closure are testable.
///
/// The shim's rigid shape (its own documented contract, relied on by `minimal-shim.mjs`): one
/// opening `globalThis.__rt = { name: body, … };` literal, then one top-level
/// `globalThis.__rt.name = …` assignment per member, each ending immediately before the next
/// member's leading comments.
fn shim_members_ordered(source: &str) -> (Vec<String>, BTreeMap<String, String>) {
    crate::profile::bump(crate::profile::Counter::ShimMembers);
    let lines: Vec<&str> = source.lines().collect();
    let mut order = Vec::new();
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
                 order: &mut Vec<String>,
                 members: &mut BTreeMap<String, String>| {
        if let Some((name, body)) = entry.take() {
            let body = body.join("\n");
            let body = body.trim_end().trim_end_matches(',');
            let statement = format!("globalThis.__rt.{name} = {body};");
            if !members.contains_key(&name) {
                order.push(name.clone());
            }
            members.insert(name, statement);
        }
    };
    for line in &lines[open + 1..close] {
        if let Some((name, rest)) = literal_entry(line) {
            flush(&mut entry, &mut order, &mut members);
            entry = Some((name, vec![rest]));
        } else if let Some((_, body)) = entry.as_mut() {
            if !line.trim_start().starts_with("//") {
                body.push(line);
            }
        }
    }
    flush(&mut entry, &mut order, &mut members);

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
        if !members.contains_key(name) {
            order.push(name.clone());
        }
        members.insert(name.clone(), lines[*from..=to].join("\n"));
    }
    (order, members)
}

#[cfg(test)]
fn shim_members(source: &str) -> BTreeMap<String, String> {
    shim_members_ordered(source).1
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
        let cost = Cost { weight_alloc: 24 };
        assert_eq!(cost.alloc_term(&sites), 24 * (1 + 4));
        assert_eq!(cost.alloc_term(&JsAllocSites::default()), 0);
    }

    /// `compact_decl` measures the shortened compact print, not the lowering's own local names.
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
        let measured = compact_decl(&decl).len() as u64;
        assert!(measured < raw, "{measured} vs {raw}");
        // Measured on a clone: the declaration itself keeps its names.
        assert!(
            jsast::program_to_string(std::slice::from_ref(&decl)).contains("a_very_long_parameter")
        );
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

    #[test]
    fn plain_vec_closure_excludes_byte_view_machinery() {
        let bytes = helper_bytes();
        assert!(
            !bytes.contains_key("_alloc_byte_proxy"),
            "eager proxy helper must stay deleted from runtime/shim.js"
        );
        let vec_direct: BTreeSet<String> = [
            "alloc",
            "alloc_zeroed",
            "realloc",
            "dealloc",
            "unscale",
            "unscale_scalar",
            "unscale_uninit",
            "retype",
            "scale",
            "slice_from_parts",
            "byte_slice_ref",
            "copy",
            "copy_nonoverlapping",
            "write_bytes",
            "ptr_eq",
            "ptr_cmp",
            "addr",
            "offset",
            "thin",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let closure = helper_closure(&vec_direct);
        for forbidden in [
            "_bv_enable",
            "_byte_view",
            "_byte_load",
            "_byte_store",
            "_prep_copy",
            "_unwrap_view",
            "_alloc_byte_proxy",
        ] {
            assert!(
                !closure.contains(forbidden),
                "plain Vec/Box helper set unexpectedly pulled `{forbidden}`: {closure:?}"
            );
        }
        for trigger in ["scale_scalar", "byte_sign", "read_byte", "write_byte"] {
            let direct: BTreeSet<String> = [trigger.to_owned()].into_iter().collect();
            let trigger_closure = helper_closure(&direct);
            assert!(
                trigger_closure.contains("_bv_enable"),
                "`{trigger}` must pull `_bv_enable`: {trigger_closure:?}"
            );
        }
    }
}
