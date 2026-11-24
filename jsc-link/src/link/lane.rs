//! The lane driver: which lowering each item is printed from.
//!
//! v2 keeps v1's printed JavaScript in every object file beside the JIR-S body, so choosing a lane
//! is a **per item** decision made here, at link time, and never a mode the whole compilation
//! commits to. For each reachable item that carries a decoded body and whose class has been
//! promoted, [`run`] asks [`crate::lower::lower_item`] for a declaration. On `Ok` the item's
//! printed text, refs, map, `.d.ts` and zombies are replaced; on `Err` the item keeps the text v1
//! printed and the decline is counted. Everything downstream — `link_dedup`, `order`,
//! `print_units`, chunking, the source map, the `.d.ts` — runs on a table it cannot tell apart
//! from a baseline one, because a [`LinkItem`] is a [`LinkItem`] whichever lane filled it in.
//!
//! # Reference validation
//!
//! The driver recomputes free identifier uses from each lowered declaration. Lexically bound
//! locals, properties and labels do not create dependencies, even when another item has the
//! same name. JavaScript builtins remain free names but do not normally name linked items.
//!
//! A lowering may remove dependencies or add references to prepared items. An unprepared item
//! reference or an undeclared compiler stub is a hard error. Final emitted references determine
//! reachability after lowering, so preparation alone does not retain an unused body.
//!
//! # Gates
//!
//! An item is offered to the lowerer when all of these hold:
//!
//! * it carries a JIR-S body that [`crate::link::decode_reachable_bodies`] decoded (a synthesized
//!   item — the allocator shim, a `global_asm` block, a link-step stub — has none, and so is never
//!   offered and never counted as a fallback);
//! * every [`ItemClass`] flag it carries is in `js-lane-classes` (default: all of them). This is
//!   the promotion order of `checkpoints/phase-2-briefs.md` as an option, one class at a time. An
//!   item with no flags at all is a subset of every set and is therefore always offered;
//! * its name is not in `examples/jir-allowlist.txt`;
//! * `js-lane-only`, when given, names it.
//!
//! # The sidecar
//!
//! `js-lane=shadow` and `js-lane=jir` write `<out>.lane.json` beside the program: one row per
//! offered item with the lane it took, the decline that sent it back, and the bytes each lowering
//! spent on it. `scripts/lane-shadow-test.sh` reads it for the fallback histogram; the per item
//! `bytes_baseline`/`bytes_jir` pair is what a size claim about the lane has to be made from, and
//! `bytes_min` is that item printed compact with its locals shortened, so
//! a size claim about ONE item does not need `js-minify` on the whole build. The run also carries
//! `passes` (how many items each pass changed), `abi` (the flat direct-call plan) and
//! `plan_retries`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::diag::Sink;
use crate::item::{item_map, prologue, write_program, JsItem, JsName, LinkItem};
use crate::jir::decline::{Decline, LowerDecline, OpClass};
use crate::jir::repr::{Func, OpKind, RTerm, ValueId};
use crate::jir::semantic::{BinaryOp, BodySemantic, CallSemantic, Callee, HostPayload, ItemId};
use crate::jir::summary::ItemClass;
use crate::jir::types::TypeKind;
use crate::jsast;
use crate::lane_report::{push_string, ItemLane};
use crate::lower::js::host::{DropGlueIndex, ItemEntry, ItemIndex, KeyedPurity, KeyedRowCapture};
use crate::lower::js::unsize::VtableIndex;
use crate::lower::{lower_item, LowerInput, LoweredItem};
use crate::opt::OptStats;
use crate::opts::{Lane, OptimizerPass};
use crate::plan::abi_flat::AbiPlan;

/// The schema of `<out>.lane.json`. Bumped when a key changes meaning.
///
/// 1 → 2: the lane runs the optimizer, so a row carries `bytes_min` and the `alloc` block, and
/// the run carries `passes`, `abi` and `plan_retries`.
/// 2 → 3: `passes` splits into `would_apply` and `applied`, and `abi` carries `applied`, so
/// `js-opt=report` can say what `js-opt=on` would do while printing what `js-opt=off` prints.
/// 3 → 4: the run carries `repr`, the representation plan's gate accounting (Phase 4, D12):
/// candidate/legal/cost-positive/committed counts, the per-region decisions, and the
/// adapter/version/helper spend. `null` when the `native-representations` analysis did not run.
/// 4 → 5: the run carries the O group's accounting (Phase 5, D5): `summaries`
/// (`items`/`validated`/`top`), `inline` (`applied`/`declined_budget`/`bytes_before`/
/// `bytes_after`), `devirt` (`applied`), `range` (`asserts_removed`), and `witnesses`
/// (`recorded`/`failed`); `passes.would_apply`/`applied` gain `inline` and `devirt` counts.
/// 5 -> 6: `repr` separates `planned`, `validated`, and `final_emitted`; final JavaScript literal
/// allocation accounting is added per item.
/// 6 -> 7: `physical` publishes the report-only `ValueShape`, `CallAbi`, `StoragePlan`, and
/// `DropPlan` selected by the existing ABI and representation machinery.
/// 7 -> 8: final-JavaScript loop sites name their originating JIR value and site, Rust type,
/// physical shape, and materialization boundary; range reporting counts aligned-prefix
/// comparisons separately from removed asserts.
/// 8 -> 9: `physical.decisions` records a selected state or explicit reason for every
/// aggregate-bearing item considered by the physical ABI planner.
/// 9 -> 10: every physical decision carries validated cross-crate summary and caller evidence.
/// 10 -> 11: physical decisions carry exact profile-aware minified, gzip, Brotli, and allocation
/// economics for planned, validated, and final-emitted programs.
/// 11 -> 12: `physical` records report-only exact numeric storage and executable cleanup choices,
/// and `async` records the versioned post-transform coroutine facts available to Phase 8.
/// 12 -> 13: storage distinguishes constant initial reservation from proven final length, the
/// physical mode names compiler-link artifact economics, and async suspension counts are
/// explicitly estimated with their derivation.
/// 13 -> 14: `slice_iter.loops_lowered` counts exact standard slice-adapter loops replaced by
/// checked scalar cursors.
/// 14 -> 15: `slice_iter` separates verified opportunities from loops retained in final output,
/// globally and per item.
/// 15 -> 16: each item reports fixed host argument-pack opportunities, retention, and declines.
/// 16 -> 17: inlining reports cross-object sites and semantic, metadata, and cost declines.
const LANE_SIDECAR_SCHEMA: u32 = 17;

/// The names that never take the JIR lane, whatever their class. See the file's own header.
const ALLOWLIST: &str = include_str!("../../../examples/jir-allowlist.txt");

/// What the driver did with one item it offered to the lowerer.
#[derive(Clone, Debug)]
pub(crate) struct LaneOutcome {
    pub name: JsName,
    pub class: ItemClass,
    /// [`ItemLane::Jir`] or [`ItemLane::Fallback`]; an item that was never offered has no row.
    pub lane: ItemLane,
    /// Why the item fell back. `None` exactly when `lane` is [`ItemLane::Jir`].
    pub decline: Option<Decline>,
    /// Whether the lowering reaches exactly what the baseline reached — the linkable subset of
    /// `refs`, which is the only part the two lanes are held to.
    ///
    /// `false` on an accepted item means the lowering proved something dead (or spells a
    /// prepared name the baseline spelled another way), and the link step walks reachability
    /// again so the program describes what is actually printed
    /// ([`LaneRun::references_changed`]). `false` on a *refused* one is the hard error that has
    /// already been reported; the row records it so the sidecar shows which item it was.
    pub refs_equal: bool,
    pub bytes_baseline: usize,
    /// The bytes the JIR lowering printed, or 0 for an item that never produced any.
    pub bytes_jir: usize,
    /// The bytes the item takes printed compact (`jsast::Style::Compact`) with its locals
    /// shortened (`minify::shorten_locals_for_measurement`), always measured.
    ///
    /// A size claim about ONE item otherwise needs `js-minify` on the whole build, which changes
    /// every other item too. The shortening is done on a clone and is never what a build prints.
    pub bytes_min: usize,
    /// The item's alloc-site accounting as JSON, when the analysis ran (W4).
    pub alloc_json: Option<String>,
    /// Fresh literals in the final lowered JavaScript AST (schema 11).
    pub js_alloc_json: Option<String>,
    /// Exact slice-adapter loops verified on the optimizer's working function.
    pub host_args: crate::opt::host_args::HostArgsStats,
    pub slice_iterator_would_lower: u32,
    /// Those loops retained in the item's final emitted lowering.
    pub slice_iterator_final_emitted: u32,
}

/// How many items each pass changed. One half of the sidecar's `passes` block.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PassCounts {
    sroa: usize,
    ast_dce: usize,
    inline: usize,
    devirt: usize,
}

/// The sidecar's `passes` block: what the passes *would* do and what they did.
///
/// The two are equal under `js-opt=on`. Under `js-opt=report` the passes run over a clone of each
/// function and the rewrite is thrown away (`opt::optimize`), so `would_apply` is what `on` would
/// have changed and `applied` is all zero — the program is the one `js-opt=off` prints. Under
/// `js-opt=off` nothing runs and both are zero.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Passes {
    would_apply: PassCounts,
    applied: PassCounts,
}

/// The O group's whole-map accounting (Phase 5, D5): what the interprocedural passes did — or,
/// under `js-opt=report`, would have done on the clone, for the sidecar's optimizer blocks.
#[derive(Clone, Debug, Default)]
pub(crate) struct OGroup {
    /// How many bodies got an effects summary, how many the post-group revalidation confirmed,
    /// and how many sit at top (unusable) after it.
    summaries_items: u32,
    summaries_validated: u32,
    summaries_top: u32,
    inline: crate::opt::inline::InlineStats,
    devirt: crate::opt::devirtualize::DevirtStats,
    /// The items the group's own dce run changed (the pure-call deletions ride it).
    dce_items: BTreeSet<JsName>,
    /// Witnesses the group's dce runs recorded and failed (inline/devirt carry their own).
    witnesses_recorded: u32,
    witnesses_failed: u32,
    /// Items whose validated effects summary proved `!may_unwind`.
    nonthrowing_items: BTreeSet<JsName>,
}

/// One region's row in the sidecar's `repr.decisions` list: what the solver chose for it.
///
/// D12's shape exactly: `{subject, family, chosen, alternatives, decline?}`. A committed region
/// has no `decline`; a candidate that stayed baseline names the [`Decline`] code (with its
/// numbers, for `cost.no-improvement`) that stopped it.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReprDecision {
    /// The region, named by its representative value: `<item>/v<id>`.
    pub subject: String,
    /// The solve family that decided it: `carrier` (F1), `vec` (F2) or `string` (F3).
    pub family: String,
    /// The carrier the region ended on.
    pub chosen: String,
    /// The carriers the family offered it, the chosen one included.
    pub alternatives: Vec<String>,
    /// Why a not-committed region was stopped, as the decline code plus its numbers.
    pub decline: Option<String>,
    /// Items containing values in the region. Internal final-reachability accounting only.
    pub items: BTreeSet<JsName>,
}

/// One exported slot's published representation: the sidecar's `repr.exports` row — v2's
/// export-representation contract (an outside caller owes the published carrier; `baseline`
/// where the slot did not commit).
#[derive(Clone, Debug, Default)]
pub(crate) struct ReprExport {
    /// The slot, named by its value node: `<item>/<slot>[.path]`.
    pub subject: String,
    /// The committed carrier the export answers with.
    pub chosen: String,
}

/// The sidecar's `repr` block: the representation plan's gate accounting (Phase 4, D12).
///
/// Schema 7 reports three stages. `planned` describes the graph and approximate min-cut proposal;
/// `validated` describes regions that passed exact whole-program D9 measurement and the
/// transaction; `final_emitted` is recomputed after final reachability and is zero in report mode.
/// A *candidate* is one maximal same-family value region with at least one candidate-typed value,
/// and *legal* has at least one finite native labeling. The historical Phase 4 gate compared
/// validated commits with cost-positive regions; zero cost-positive regions do not pass it.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReprBlock {
    /// How many solve families ran (F1 carrier, F2 vec, F3 string).
    pub families: u32,
    pub candidates: u32,
    pub legal: u32,
    pub cost_positive: u32,
    pub committed: u32,
    /// Regions the approximate min-cut proposed before exact D9 validation.
    pub planned: u32,
    /// Validated regions that survive in the final reachable output. Zero in report mode.
    pub applied: u32,
    pub decisions: Vec<ReprDecision>,
    /// How many `Convert` adapters the committed labelings spelled at edges.
    pub adapters: u32,
    /// How many `$n` native version clones the commit added.
    pub versions: u32,
    /// The shim-member bytes the committed labels newly reach (`plan::cost::helper_bytes`).
    pub helper_bytes: u64,
    /// D10b: regions blocked ONLY by item-boundary pins (`valueflow::pins`' `unknown-call`),
    /// which one `$n` clone would bypass — the clone path's build-or-record evidence.
    pub pinned_cost_positive: u32,
    /// The published export-representation contract, one row per exported candidate slot.
    pub exports: Vec<ReprExport>,
    /// Exact D9 compact emitted bytes and total weighted scores.
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub score_before: u64,
    pub score_after: u64,
    /// Whether this block belongs to the program being emitted (`on`) or a report-only proposal.
    pub emits: bool,
}

impl ReprBlock {
    /// The block as sidecar JSON.
    fn render(&self, out: &mut String) {
        let emitted_bytes = if self.emits {
            self.bytes_after
        } else {
            self.bytes_before
        };
        let _ = write!(
            out,
            "{{\"families\":{},\"planned\":{{\"candidates\":{},\"legal\":{},\
             \"would_commit\":{},\"pinned_cost_positive\":{}}},\
             \"validated\":{{\"cost_positive\":{},\"would_commit\":{},\"adapters\":{},\
             \"versions\":{},\"helper_bytes\":{},\"bytes_before\":{},\"bytes_after\":{},\
             \"score_before\":{},\"score_after\":{}}},\
             \"final_emitted\":{{\"applied\":{},\"bytes\":{}}},\"decisions\":[",
            self.families,
            self.candidates,
            self.legal,
            self.planned,
            self.pinned_cost_positive,
            self.cost_positive,
            self.committed,
            self.adapters,
            self.versions,
            self.helper_bytes,
            self.bytes_before,
            self.bytes_after,
            self.score_before,
            self.score_after,
            self.applied,
            emitted_bytes,
        );
        for (index, decision) in self.decisions.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"subject\":");
            push_string(out, &decision.subject);
            out.push_str(",\"family\":");
            push_string(out, &decision.family);
            out.push_str(",\"chosen\":");
            push_string(out, &decision.chosen);
            out.push_str(",\"alternatives\":[");
            for (position, alternative) in decision.alternatives.iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                push_string(out, alternative);
            }
            out.push_str("],\"decline\":");
            match &decision.decline {
                Some(decline) => push_string(out, decline),
                None => out.push_str("null"),
            }
            out.push('}');
        }
        out.push_str("],\"exports\":[");
        for (index, export) in self.exports.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"subject\":");
            push_string(out, &export.subject);
            out.push_str(",\"chosen\":");
            push_string(out, &export.chosen);
            out.push('}');
        }
        out.push_str("]}");
    }

    /// Recomputes the applied count from the final reachable item set.
    fn retain_reachable(&mut self, reachable: &BTreeSet<&JsName>) {
        if !self.emits {
            self.applied = 0;
            return;
        }
        self.applied = self
            .decisions
            .iter()
            .filter(|decision| {
                decision.decline.is_none()
                    && decision.chosen != "baseline"
                    && decision.items.iter().any(|item| reachable.contains(item))
            })
            .count() as u32;
    }
}

/// Everything one lane pass decided, in name order.
#[derive(Clone, Debug, Default)]
pub(crate) struct LaneRun {
    outcomes: Vec<LaneOutcome>,
    passes: Passes,
    /// The O group's accounting; all zero when the group did not run.
    o: OGroup,
    /// Bounds-check asserts the range pass removed, summed over the items.
    range_asserts: u64,
    /// Aligned slice-prefix comparisons the range pass folded, summed over the items.
    range_comparisons: u64,
    /// Witnesses recorded and failed, the per-item passes and the O group together.
    witnesses_recorded: u64,
    witnesses_failed: u64,
    /// The flat-call ABI plan as JSON, when one was made (W3).
    abi_json: Option<String>,
    /// The report-only physical plan derived from the selected ABI and carriers.
    physical_json: Option<String>,
    /// The report-only async graph facts derived from decoded post-transform types.
    async_json: Option<String>,
    /// The representation plan's accounting, when its analysis ran (W4). Empty until the solver
    /// lands; `None` renders as `"repr":null`.
    repr: Option<ReprBlock>,
    /// How many times the run restarted with an empty ABI plan. See [`run`].
    plan_retries: u32,
    /// Whether the lane added items to the table (the `$rN` spill registers), which is what makes
    /// the link step walk reachability again.
    added_items: bool,
    /// Whole-program lazy byte-view policy.
    byte_views: crate::analysis::byte_views::ByteViewPlan,
    /// Whether universal flat `Vec<T, Global>` representation was active.
    native_vec_enabled: bool,
}

impl LaneRun {
    /// Whether universal flat `Vec<T, Global>` representation was active.
    pub(crate) fn native_vec_enabled(&self) -> bool {
        self.native_vec_enabled
    }
    /// The items the lowerer declined. The exit gate's number.
    pub(crate) fn fallback_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.lane == ItemLane::Fallback)
            .count()
    }

    /// The row for `name`, for the lane report's `lane` field.
    pub(crate) fn outcome(&self, name: &JsName) -> Option<&LaneOutcome> {
        self.outcomes
            .binary_search_by(|outcome| outcome.name.as_str().cmp(name.as_str()))
            .ok()
            .map(|index| &self.outcomes[index])
    }

    /// Drops the rows of items that are no longer in the program.
    ///
    /// `link_dedup` runs after the lane and merges duplicates, so an item the lane lowered can
    /// stop being printed under its own name. The sidecar describes the program that came out.
    pub(crate) fn retain_reachable(&mut self, reachable: &[JsName]) {
        let live: BTreeSet<&JsName> = reachable.iter().collect();
        self.outcomes.retain(|outcome| live.contains(&outcome.name));
        if let Some(repr) = &mut self.repr {
            repr.retain_reachable(&live);
        }
    }

    /// `js-str=bytes` moved the lowered body of an export to `to` and put a facade under its
    /// name (`link.rs::byte_string_facades`); the outcome follows the body.
    pub(crate) fn rename(&mut self, from: &JsName, to: &JsName) {
        for outcome in &mut self.outcomes {
            if &outcome.name == from {
                outcome.name = to.clone();
            }
        }
    }

    /// Whether the lane added items to the program, so the link step re-walks reachability.
    pub(crate) fn added_items(&self) -> bool {
        self.added_items
    }

    /// Whether the program may alias wide allocations through byte pointers/views.
    pub(crate) fn wide_byte_views(&self) -> bool {
        self.byte_views.raw_byte_derefs
    }

    /// Whether any item the lane printed reaches something other than what its baseline reached.
    ///
    /// The driver accepts a lowering with fewer linkable references than the baseline's — that
    /// is what a folded branch looks like — and one that names something the walk already
    /// prepared. Either way the `refs` the reachability walk ran on are no longer the item's, so
    /// the link step walks again: without that, an item nothing calls any more stays in the
    /// program.
    pub(crate) fn references_changed(&self) -> bool {
        self.outcomes
            .iter()
            .any(|outcome| outcome.lane == ItemLane::Jir && !outcome.refs_equal)
    }

    /// `<out>.lane.json`: what each offered item did, sorted by name.
    pub(crate) fn render(&self, lane: Lane) -> String {
        let mut out = String::with_capacity(128 * self.outcomes.len() + 128);
        let slice_iterator_would_lower: u64 = self
            .outcomes
            .iter()
            .map(|outcome| u64::from(outcome.slice_iterator_would_lower))
            .sum();
        let slice_iterator_final_emitted: u64 = self
            .outcomes
            .iter()
            .map(|outcome| u64::from(outcome.slice_iterator_final_emitted))
            .sum();
        let _ = write!(out, "{{\"schema\":{LANE_SIDECAR_SCHEMA},\"lane\":");
        push_string(&mut out, lane.as_str());
        let _ = write!(
            out,
            ",\"fallback_count\":{},\"passes\":{{\
             \"would_apply\":{{\"sroa\":{},\"ast_dce\":{},\"inline\":{},\"devirt\":{}}},\
             \"applied\":{{\"sroa\":{},\"ast_dce\":{},\"inline\":{},\"devirt\":{}}}}},\
             \"summaries\":{{\"items\":{},\"validated\":{},\"top\":{}}},\
             \"inline\":{{\"applied\":{},\"cross_object_applied\":{},\"declined\":{},\"declined_budget\":{},\"bytes_before\":{},\
             \"bytes_after\":{}}},\
             \"devirt\":{{\"applied\":{}}},\
             \"range\":{{\"asserts_removed\":{},\"comparisons_folded\":{}}},\
             \"slice_iter\":{{\"would_lower\":{},\"final_emitted\":{}}},\
             \"witnesses\":{{\"recorded\":{},\"failed\":{}}},\
             \"plan_retries\":{},\"abi\":{},\"physical\":{},\"async\":{},\"repr\":",
            self.fallback_count(),
            self.passes.would_apply.sroa,
            self.passes.would_apply.ast_dce,
            self.passes.would_apply.inline,
            self.passes.would_apply.devirt,
            self.passes.applied.sroa,
            self.passes.applied.ast_dce,
            self.passes.applied.inline,
            self.passes.applied.devirt,
            self.o.summaries_items,
            self.o.summaries_validated,
            self.o.summaries_top,
            self.o.inline.applied,
            self.o.inline.cross_object_applied,
            self.o.inline.declines_json(),
            self.o.inline.declined_budget,
            self.o.inline.bytes_before,
            self.o.inline.bytes_after,
            self.o.devirt.applied,
            self.range_asserts,
            self.range_comparisons,
            slice_iterator_would_lower,
            slice_iterator_final_emitted,
            self.witnesses_recorded,
            self.witnesses_failed,
            self.plan_retries,
            self.abi_json.as_deref().unwrap_or("null"),
            self.physical_json.as_deref().unwrap_or("null"),
            self.async_json.as_deref().unwrap_or("null"),
        );
        match &self.repr {
            Some(repr) => repr.render(&mut out),
            None => out.push_str("null"),
        }
        out.push_str(",\"items\":[");
        for (index, outcome) in self.outcomes.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"name\":");
            push_string(&mut out, outcome.name.as_str());
            out.push_str(",\"class\":[");
            for (position, name) in outcome.class.names().iter().enumerate() {
                if position > 0 {
                    out.push(',');
                }
                push_string(&mut out, name);
            }
            out.push_str("],\"lane\":");
            push_string(&mut out, outcome.lane.as_str());
            out.push_str(",\"decline\":");
            match &outcome.decline {
                Some(decline) => push_string(&mut out, decline.code()),
                None => out.push_str("null"),
            }
            let _ = write!(
                out,
                ",\"refs_equal\":{},\"bytes_baseline\":{},\"bytes_jir\":{},\"bytes_min\":{},\
                 \"slice_iter\":{{\"would_lower\":{},\"final_emitted\":{}}},\
                 \"host_args\":{},\"alloc\":{},\"js_alloc\":{}}}",
                outcome.refs_equal,
                outcome.bytes_baseline,
                outcome.bytes_jir,
                outcome.bytes_min,
                outcome.slice_iterator_would_lower,
                outcome.slice_iterator_final_emitted,
                outcome.host_args.to_json(),
                outcome.alloc_json.as_deref().unwrap_or("null"),
                outcome.js_alloc_json.as_deref().unwrap_or("null"),
            );
        }
        out.push_str("]}\n");
        out
    }
}

/// Writes the sidecar beside the program.
pub(crate) fn write_sidecar(dcx: &Sink, path: &Path, run: &LaneRun, lane: Lane) {
    let path = sidecar_path(path);
    let mut text = run.render(lane);
    // `RCGJS_COST_PROFILE=1` only: the compile-cost figures so far, as the object's last member.
    if crate::profile::enabled() {
        if let Some(body) = text.strip_suffix("}\n") {
            text = format!("{body},\"cost_profile\":{}}}\n", crate::profile::json());
        }
    }
    if let Err(err) = std::fs::write(&path, text) {
        dcx.err(format!("error writing `{}`: {err}", path.display()));
    }
}

/// `<out>.lane.json` for an output path.
pub(crate) fn sidecar_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lane.json");
    path.with_file_name(name)
}

/// `<out>.jir.js` for an output path: where the shadow lane's second program goes.
pub(crate) fn shadow_path(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".jir.js");
    path.with_file_name(name)
}

/// Lowers every promoted reachable item in `table` from its JIR-S body, in place.
///
/// Called once per lane pass, immediately after the bodies are decoded and before anything
/// downstream reads the printed text.
///
/// # Three phases
///
/// A whole-program pass needs every item's JIR-R at once, and a per-item pass needs to run after
/// it, so the driver is no longer one loop:
///
/// * **A** builds the [`Func`] of every offered function ([`crate::lower::build_func`]). A `static`
///   and a vtable have no `Func` and skip to C.
/// * **B** plans the flat direct-call ABI over all of them and rewrites callees and callers.
/// * **C** runs [`crate::opt::optimize`] over each `Func` and prints it
///   ([`crate::lower::lower_func`]).
///
/// Phase C produces declarations without touching the table, and the whole of it is applied
/// afterwards. That is what makes the **retry** affordable: an item the ABI plan changed that then
/// declines in C would leave its callers printing calls in the old shape, which is a miscompile, so
/// the run restarts from A with an **empty** plan. One retry is a fixed point — the empty plan
/// touches nothing, so nothing it touched can decline.
pub(crate) fn run(
    dcx: &Sink,
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    coroutine_sources: &crate::plan::async_graph::SourceCaptures,
) -> LaneRun {
    let opts = crate::opts::get();
    let fault = abi_fault();
    run_with_opts(
        dcx,
        table,
        reachable,
        coroutine_sources,
        opts,
        fault.as_deref(),
    )
}

fn merge_o_stats(dst: &mut OGroup, src: OGroup) {
    dst.inline.applied += src.inline.applied;
    dst.inline.cross_object_applied += src.inline.cross_object_applied;
    dst.inline.declined_budget += src.inline.declined_budget;
    for (reason, count) in src.inline.declined {
        *dst.inline.declined.entry(reason).or_default() += count;
    }
    dst.inline.bytes_before += src.inline.bytes_before;
    dst.inline.bytes_after += src.inline.bytes_after;
    dst.inline.items.extend(src.inline.items);
    dst.devirt.applied += src.devirt.applied;
    dst.devirt.witnesses_recorded += src.devirt.witnesses_recorded;
    dst.devirt.witnesses_failed += src.devirt.witnesses_failed;
    dst.devirt.items.extend(src.devirt.items);
    dst.dce_items.extend(src.dce_items);
    dst.witnesses_recorded += src.witnesses_recorded;
    dst.witnesses_failed += src.witnesses_failed;
}

fn run_with_opts(
    dcx: &Sink,
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    coroutine_sources: &crate::plan::async_graph::SourceCaptures,
    opts: &crate::opts::Opts,
    fault: Option<&str>,
) -> LaneRun {
    let _lane_scope = crate::profile::scope("lane.total");
    // The names the reachability walk can follow, taken before any item is replaced. Cloned
    // rather than borrowed because the loop below holds `&mut` on one item of the same table.
    let mut defined: BTreeSet<JsName> = table.keys().cloned().collect();
    // What the walk has already decoded, ordered and will print. A lowering may reference any of
    // it, whether or not the baseline did; see `apply`.
    let mut live: BTreeSet<JsName> = reachable.iter().cloned().collect();
    // Every item of the program by its semantic identity: what the host lowering resolves a
    // closure body's name and arity through (`lower/js/host.rs`).
    let (mut items, drop_glue) = item_index(table);
    let vtables = vtable_index(table);
    let original_names = offered(table, reachable, opts);
    let mut names = original_names.clone();

    // Link-time whole-program enum specialization (`EnumRepr::NullNiche` and `EnumRepr::IntTag`)
    // runs BEFORE Phase A `build_funcs` under `ReleaseSpeed` so SSA cell boxing, `SetDiscriminant`,
    // `needs_clone`, `fresh_container`, and `PointerForm::Thin -> Slot` all observe one consistent
    // `TypeTable`. Skipped completely under `Report` / `Off` / non-`ReleaseSpeed`.
    let mut enum_excluded: BTreeSet<rcgjs_object::jir::semantic::TypeId> = BTreeSet::new();
    let unspecialized_table = if opts.optimizer == crate::opts::OptimizerMode::On
        && opts.profile == Some(crate::opts::Profile::ReleaseSpeed)
    {
        Some(table.clone())
    } else {
        None
    };
    let mut enum_plan = crate::plan::enum_repr::specialize_table(
        table,
        reachable,
        &original_names,
        &enum_excluded,
        opts,
    );

    // Phase A, once: the plan is made over the same Funcs the first pass rewrites, and a retry
    // rebuilds them because the rewrite consumed them.
    let mut pristine_funcs = {
        let _scope = crate::profile::scope("lane.A.build_funcs");
        let ctx = Ctx {
            opts,
            items: &items,
            drop_glue: &drop_glue,
            vtables: &vtables,
            byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
            heap: crate::analysis::heap::HeapPlan::default(),
        };
        build_funcs(table, &original_names, &ctx)
    };
    if !enum_plan.is_empty() {
        let declined_a: Vec<JsName> = pristine_funcs
            .iter()
            .filter_map(|(n, res)| res.is_err().then_some(n.clone()))
            .collect();
        if !declined_a.is_empty() {
            if let Some(raw_table) = &unspecialized_table {
                let prev_len = enum_excluded.len();
                crate::plan::enum_repr::collect_item_signature_types(
                    raw_table,
                    &declined_a,
                    &mut enum_excluded,
                );
                if enum_excluded.len() > prev_len {
                    *table = raw_table.clone();
                    enum_plan = crate::plan::enum_repr::specialize_table(
                        table,
                        reachable,
                        &original_names,
                        &enum_excluded,
                        opts,
                    );
                    let ctx = Ctx {
                        opts,
                        items: &items,
                        drop_glue: &drop_glue,
                        vtables: &vtables,
                        byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
                        heap: crate::analysis::heap::HeapPlan::default(),
                    };
                    pristine_funcs = build_funcs(table, &original_names, &ctx);
                }
            }
        }
    }
    let empty_native_bodies = BTreeMap::new();
    let mut byte_views = crate::analysis::byte_views::analyze_reachable(
        table,
        &pristine_funcs,
        &empty_native_bodies,
        &opts.roots,
    );
    let mut heap = crate::analysis::heap::analyze_reachable(
        table,
        &pristine_funcs,
        &empty_native_bodies,
        opts,
        byte_views,
    );
    record_keyed_purity(table, &pristine_funcs, &mut items);
    let mut ctx = Ctx {
        opts,
        items: &items,
        drop_glue: &drop_glue,
        vtables: &vtables,
        byte_views,
        heap,
    };
    // Effects-summary coverage (phase 5, W5b): analysis only, stderr only, computed on demand.
    // The O group (W5e) is what will consume the summaries in the pipeline.
    if std::env::var("RCGJS_SUMMARY_REPORT").is_ok_and(|value| value == "1") {
        crate::analysis::effects::print_report(table, &pristine_funcs);
    }
    // Phase O (phase 5, D5): summaries, inline, devirt and dce run before representation planning.
    // This is the whole-program shape B1 must price. Report mode owns a planned clone while the
    // emitted map remains pristine.
    let run_o = |table: &mut BTreeMap<JsName, LinkItem>,
                 funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
                 live: &BTreeSet<JsName>,
                 ctx: &Ctx<'_>| {
        if opts.analyzes(OptimizerPass::AstDce) {
            o_group(table, funcs, ctx, live)
        } else {
            OGroup::default()
        }
    };
    let pristine_table = table.clone();
    let mut planned_funcs = pristine_funcs.clone();
    let mut o_stats =
        crate::profile::time("lane.O", || run_o(table, &mut planned_funcs, &live, &ctx));
    byte_views = crate::analysis::byte_views::analyze_reachable(
        table,
        &planned_funcs,
        &empty_native_bodies,
        &opts.roots,
    );
    heap = crate::analysis::heap::analyze_reachable(
        table,
        &planned_funcs,
        &empty_native_bodies,
        opts,
        byte_views,
    );
    ctx.byte_views = byte_views;
    ctx.heap = heap;
    let pre_repr_native_string = ctx.heap.native_string_enabled;
    let pre_repr_native_vec = ctx.heap.native_vec_enabled;

    // Phase B1: solve and commit on the planned map in both `report` and `on`. Report mode keeps
    // only the accounting; on mode prints the committed map. D9 can therefore measure the exact
    // post-O program in both modes and return identical decisions.
    let repr_ran = opts.analyzes(OptimizerPass::NativeRepresentations);
    let planned_repr = if repr_ran {
        let repr_live = crate::analysis::heap::filter_live_for_repr(
            table,
            &planned_funcs,
            &live,
            opts,
            &ctx.heap,
        );
        let repr_cx = crate::plan::representation::ReprCx {
            opts,
            items: &items,
            drop_glue: &drop_glue,
            vtables: &vtables,
            byte_views,
            heap: &ctx.heap,
            nonthrowing_glues: &o_stats.nonthrowing_items,
        };
        crate::profile::time("lane.B1.representation", || {
            crate::plan::representation::plan_and_commit(
                table,
                &mut planned_funcs,
                &repr_live,
                &repr_cx,
                true,
            )
        })
    } else {
        crate::plan::representation::ReprOutcome::default()
    };
    let planned_clones = planned_repr.clones.clone();
    let planned_body_names: Vec<JsName> = planned_repr.bodies.keys().cloned().collect();
    let planned_nested_clone = planned_repr.nested_clone.clone();
    let mut planning_table = table.clone();
    for (clone, source) in &planned_clones {
        let Some(source) = planning_table.get(source).cloned() else {
            continue;
        };
        planning_table.insert(
            clone.clone(),
            crate::plan::representation::clone_link_item(source, clone),
        );
    }
    let emitting_plans = opts.optimizer == crate::opts::OptimizerMode::On;
    if opts.analyzes(OptimizerPass::AstDce) {
        prune_noop_heap_drop_glues(&planning_table, &mut planned_funcs, &ctx.heap);
        let mut excluded: BTreeMap<JsName, Result<Func, Decline>> = BTreeMap::new();
        for name in planned_repr
            .bodies
            .keys()
            .chain(planned_repr.nested_clone.keys())
        {
            if let Some(entry) = planned_funcs.remove(name) {
                excluded.insert(name.clone(), entry);
            }
        }
        let mut post_live = live.clone();
        post_live.extend(planned_clones.keys().cloned());
        let post_o = o_group_with_mode(
            &mut planning_table,
            &mut planned_funcs,
            &ctx,
            &post_live,
            true,
            Some(&planned_repr.call_retargets),
        );
        planned_funcs.extend(excluded);
        merge_o_stats(&mut o_stats, post_o);
    }
    let mut funcs = if emitting_plans {
        planned_funcs.clone()
    } else {
        pristine_funcs.clone()
    };
    let mut repr = if emitting_plans {
        planned_repr
    } else {
        planned_repr.unapplied()
    };

    // Only an applying run offers private versions to phase C. Report mode still priced and
    // planned them above, but its emitted table and item list remain byte-identical to `off`.
    let mut work_table = if emitting_plans {
        for clone in repr.clones.keys() {
            names.push(clone.clone());
            defined.insert(clone.clone());
            live.insert(clone.clone());
        }
        names.sort();
        names.dedup();
        planning_table.clone()
    } else {
        table.clone()
    };
    byte_views = crate::analysis::byte_views::analyze_reachable(
        &work_table,
        &funcs,
        &repr.bodies,
        &opts.roots,
    );
    heap = crate::analysis::heap::analyze_reachable(
        &work_table,
        &funcs,
        &repr.bodies,
        opts,
        byte_views,
    );
    if !repr.bodies.is_empty() {
        // `repr.bodies` were emitted against `pre_repr_native_string` / `pre_repr_native_vec`;
        // keep the post-repr heap plan in lockstep so native bodies and lowered items agree.
        heap.native_string_enabled &= pre_repr_native_string;
        heap.native_vec_enabled &= pre_repr_native_vec;
    }
    ctx.byte_views = byte_views;
    ctx.heap = heap;

    // B2 plans against the same post-O, post-representation map D9 measured. Report mode records
    // that plan and empties it before emission.
    let mut abi_funcs = planned_funcs.clone();
    for clone in planned_clones.keys() {
        abi_funcs.remove(clone);
    }
    let abi_excluded: BTreeSet<crate::jir::semantic::ItemId> = planned_clones
        .values()
        .chain(planned_body_names.iter())
        .filter_map(|source| {
            pristine_funcs
                .get(source)?
                .as_ref()
                .ok()
                .map(|func| func.item)
        })
        .collect();
    let mut plan = crate::profile::time("lane.B2.abi_flat", || {
        crate::plan::abi_flat::plan_excluding(&planning_table, &abi_funcs, opts, &abi_excluded)
    });
    let abi_applied = opts.applies(OptimizerPass::Sroa);
    let mut economics = None;
    if !plan.is_empty() {
        let _scope = crate::profile::scope("lane.B2.physical");
        let mut reuse = crate::plan::lowering_reuse::Reuse::physical();
        let (selected, evaluation) = crate::plan::physical_cost::evaluate_groups(
            &planning_table,
            &planned_funcs,
            &plan,
            opts,
            abi_applied,
            |candidate| {
                physical_program_metrics(
                    &planning_table,
                    &planned_funcs,
                    candidate,
                    &ctx,
                    &planned_nested_clone,
                    &mut reuse,
                )
            },
        );
        plan = selected;
        economics = Some(evaluation);
    }
    let abi_clones = plan.clones().clone();
    let mut physical_table = planning_table.clone();
    let mut physical_funcs = planned_funcs.clone();
    plan.install_clones(&mut physical_table, &mut physical_funcs);
    let planned_physical = crate::plan::physical::PhysicalPlan::of(
        &physical_table,
        &physical_funcs,
        &plan,
        opts.typed_storage && opts.optimizer == crate::opts::OptimizerMode::On,
    )
    .with_economics(economics.clone());
    if emitting_plans {
        plan.install_clones(&mut work_table, &mut funcs);
        for clone in abi_clones.keys() {
            names.push(clone.clone());
            defined.insert(clone.clone());
            live.insert(clone.clone());
        }
        names.sort();
        names.dedup();
    }
    // `js-opt=report` plans so it can report the plan, and prints the program `js-opt=off`
    // prints: the would-be plan is recorded here and then emptied, before phase B, so nothing is
    // rewritten, no spill register is added and no item is touched. Under `js-opt=on` the plan is
    // reported after the loop instead, since a retry is what the sidecar has to describe.
    let mut reported = None;
    if !abi_applied {
        reported = plan.json(false);
        plan = plan.emptied();
    }
    // A spill register is an item the lane is about to add, so a lowering that names one names
    // something the link step has prepared. Known before phase C prints anything, because that is
    // where a lowering is checked against these two sets.
    for register in plan.spill_names() {
        defined.insert(register.clone());
        live.insert(register.clone());
    }
    for helper in crate::lower::js::i64_pair::HELPER_NAMES {
        let helper_name = JsName::new((*helper).to_owned());
        if !table.contains_key(&helper_name) {
            defined.insert(helper_name.clone());
            live.insert(helper_name);
        }
    }
    let mut bstr_const_items = Vec::new();
    if crate::lower::bstr::enabled() {
        for (name, item) in table.iter_mut() {
            if item.kind == rcgjs_object::object::ItemKind::Const
                && name.as_str().starts_with("loc$h")
                && live.contains(name)
            {
                if let Some(rewritten) =
                    super::hoist_consts::rewrite_bstr_location_const_item(&item.js)
                {
                    item.js = rewritten;
                }
            }
        }
        for (name, item) in table.iter() {
            if item.kind == rcgjs_object::object::ItemKind::Const
                && name.as_str().starts_with("s$h")
                && live.contains(name)
            {
                if let Some(bs_item) =
                    super::hoist_consts::synthesize_bstr_const_item(name.as_str(), &item.js)
                {
                    if !table.contains_key(&bs_item.name) && !defined.contains(&bs_item.name) {
                        defined.insert(bs_item.name.clone());
                        live.insert(bs_item.name.clone());
                        bstr_const_items.push(bs_item);
                    }
                }
            }
        }
    }
    // ES2018 spells a BigInt constant as a call that parses its digits whenever it runs, so phase
    // C names a module-level binding for each one instead (`lower/bigint.rs`). The prefix is
    // allocated around every name the program spells, its first-pass clones and registers
    // included; a retry spells a subset of those.
    let bigint_names = if opts.ecma_target == crate::opts::EcmaTarget::Es2018 {
        crate::lower::bigint::BigIntNames::allocate(&work_table, plan.spill_names())
    } else {
        None
    };
    let mut plan_retries = 0;
    let mut enum_retries = 0u32;
    let mut ready_allowed = true;
    let mut physical = planned_physical;
    let (mut lowered, async_plan) = loop {
        let rewrite_scope = crate::profile::scope("lane.rewrite");
        if abi_applied {
            physical = crate::plan::physical::PhysicalPlan::of(
                &work_table,
                &funcs,
                &plan,
                opts.typed_storage && opts.optimizer == crate::opts::OptimizerMode::On,
            )
            .with_economics(economics.clone());
        }
        let mut async_plan =
            crate::plan::async_graph::AsyncPlan::of(&work_table, &funcs, coroutine_sources, opts);
        if !ready_allowed {
            async_plan.ready.disable();
        }
        physical
            .ready_futures
            .clone_from(&async_plan.ready.selected);
        // Phase B2.
        plan.rewrite(&mut funcs, &work_table);
        drop(rewrite_scope);
        // Phase C.
        let report_probe = (opts.optimizer == crate::opts::OptimizerMode::Report)
            .then_some((&planning_table, &planned_funcs));
        let (lowered, retry) = crate::profile::time("lane.C.lower_all", || {
            lower_all(
                &work_table,
                &names,
                &ctx,
                funcs,
                report_probe,
                &plan,
                &repr,
                &physical,
                &defined,
                &live,
                fault,
                bigint_names.as_ref(),
            )
        });
        let enum_conflict_items: Vec<JsName> = if !enum_plan.is_empty() && enum_retries < 2 {
            let declined_c: Vec<JsName> = lowered
                .iter()
                .filter_map(|(n, res, _, _)| res.is_err().then_some(n.clone()))
                .collect();
            if declined_c.is_empty() {
                Vec::new()
            } else if let Some(raw_table) = &unspecialized_table {
                let mut sig_tys = BTreeSet::new();
                crate::plan::enum_repr::collect_item_signature_types(
                    raw_table,
                    &declined_c,
                    &mut sig_tys,
                );
                if sig_tys.iter().any(|t| enum_plan.decisions.contains_key(t)) {
                    declined_c
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        if !enum_conflict_items.is_empty()
            || (retry
                && (!plan.is_empty()
                    || !repr.touched.is_empty()
                    || !physical.ready_futures.is_empty()))
        {
            let _scope = crate::profile::scope("lane.retry");
            ready_allowed = false;
            // BOTH plans are emptied (D1): the retried run rebuilds pristine Funcs and rewrites
            // nothing, so nothing it touched can decline. Each keeps its refusal accounting, so
            // the sidecar still says what was considered on a run that kept none of it.
            plan = plan.emptied();
            repr = repr.emptied();
            economics = economics.as_ref().map(|economics| economics.after_retry());
            plan_retries += 1;
            for clone in planned_clones.keys() {
                defined.remove(clone);
                live.remove(clone);
            }
            for clone in abi_clones.keys() {
                defined.remove(clone);
                live.remove(clone);
            }
            names.clone_from(&original_names);
            if !enum_conflict_items.is_empty() {
                enum_retries += 1;
                if let Some(raw_table) = &unspecialized_table {
                    crate::plan::enum_repr::collect_item_signature_types(
                        raw_table,
                        &enum_conflict_items,
                        &mut enum_excluded,
                    );
                    *table = raw_table.clone();
                    enum_plan = crate::plan::enum_repr::specialize_table(
                        table,
                        reachable,
                        &original_names,
                        &enum_excluded,
                        opts,
                    );
                } else {
                    *table = pristine_table.clone();
                }
            } else {
                *table = pristine_table.clone();
            }
            funcs = build_funcs(table, &original_names, &ctx);
            // The O group re-runs on the pristine table and Funcs (both `run_o` and `post_o`,
            // matching a plan-less run), so the retried program is byte-identical to
            // `js-jir-flat-abi=off`.
            o_stats = run_o(table, &mut funcs, &live, &ctx);
            work_table = table.clone();
            if emitting_plans && opts.analyzes(OptimizerPass::AstDce) {
                let post_o =
                    o_group_with_mode(&mut work_table, &mut funcs, &ctx, &live, true, None);
                merge_o_stats(&mut o_stats, post_o);
            }
            byte_views = crate::analysis::byte_views::analyze_reachable(
                &work_table,
                &funcs,
                &repr.bodies,
                &opts.roots,
            );
            heap = crate::analysis::heap::analyze_reachable(
                &work_table,
                &funcs,
                &repr.bodies,
                opts,
                byte_views,
            );
            if !repr.bodies.is_empty() {
                heap.native_string_enabled &= pre_repr_native_string;
                heap.native_vec_enabled &= pre_repr_native_vec;
            }
            ctx.byte_views = byte_views;
            ctx.heap = heap;
            continue;
        }
        break (lowered, async_plan);
    };

    // The spill registers the plan needs are items like any other, and they have to be in the
    // table and reachable before a lowering that names them is checked against either.
    let spills = plan.spill_items();
    let mut added_items = !spills.is_empty() || !repr.clones.is_empty() || !abi_clones.is_empty();
    if opts.optimizer == crate::opts::OptimizerMode::On {
        added_items |= super::hoist_consts::run(table, &mut lowered, &mut defined, &mut live);
        if let Some(names) = &bigint_names {
            for (item_name, result, _, _) in lowered.iter_mut() {
                let Ok(lowering) = result else { continue };
                if !matches!(lowering.decl, jsast::Stmt::FunctionDecl { .. }) {
                    continue;
                }
                crate::lower::bigint::hoist(&mut lowering.decl, names, &mut lowering.bigints);
                lowering.refs = refs_of(&lowering.decl, item_name);
            }
        }
    }
    for spill in spills {
        // Never `insert`: a register's name was allocated around every name the program already
        // spells (`plan::abi_flat::allocate_spill_names`), and overwriting an item that the
        // program does define would delete it. Unreachable, and a hard error if it ever is not.
        match table.entry(spill.name.clone()) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(spill);
            }
            std::collections::btree_map::Entry::Occupied(slot) => dcx.err(format!(
                "rustc_codegen_js: the flat direct-call ABI allocated the spill register name                  `{}`, which the program already defines; this is a compiler bug, please report it",
                slot.key().as_str()
            )),
        }
    }

    // One `const` per BigInt constant the lowerings name, plus the `BigInt` test and the throwing
    // function the guarded uses name, the same way: prepared before `apply` checks a lowering
    // against `defined` and `live`, and never over an item the program defines (the prefix was
    // allocated around all of them). A binding no installed item ends up naming is unreachable and
    // is not printed.
    let constants: BTreeSet<&String> = lowered
        .iter()
        .filter_map(|(_, result, _, _)| result.as_ref().ok())
        .flat_map(|lowering| &lowering.bigints)
        .collect();
    let mut added_bigints = false;
    let support = match &bigint_names {
        Some(names) if !constants.is_empty() => names.support().to_vec(),
        _ => Vec::new(),
    };
    let hoisted = constants
        .into_iter()
        .filter_map(|digits| bigint_names.as_ref().and_then(|names| names.item(digits)));
    for constant in support.into_iter().chain(hoisted) {
        defined.insert(constant.name.clone());
        live.insert(constant.name.clone());
        match table.entry(constant.name.clone()) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(constant);
                added_bigints = true;
            }
            std::collections::btree_map::Entry::Occupied(slot) => dcx.err(format!(
                "rustc_codegen_js: the BigInt constant hoist allocated the name `{}`, which the \
                 program already defines; this is a compiler bug, please report it",
                slot.key().as_str()
            )),
        }
    }
    let mut added_bstr_consts = false;
    for bs_item in bstr_const_items {
        if let std::collections::btree_map::Entry::Vacant(slot) = table.entry(bs_item.name.clone())
        {
            slot.insert(bs_item);
            added_bstr_consts = true;
        }
    }
    let added_items = added_items || added_bigints || added_bstr_consts;

    let mut passes = Passes::default();
    let mut range_asserts: u64 = 0;
    let mut range_comparisons: u64 = 0;
    let mut witnesses_recorded: u64 = u64::from(
        o_stats.witnesses_recorded
            + o_stats.inline.witnesses_recorded
            + o_stats.devirt.witnesses_recorded,
    );
    let mut witnesses_failed: u64 = u64::from(
        o_stats.witnesses_failed
            + o_stats.inline.witnesses_failed
            + o_stats.devirt.witnesses_failed,
    );
    let mut outcomes = Vec::with_capacity(lowered.len());
    for (name, result, stats, alloc) in lowered {
        range_asserts += u64::from(stats.asserts_removed);
        range_comparisons += u64::from(stats.range_comparisons_folded);
        witnesses_recorded += u64::from(stats.witnesses_recorded);
        witnesses_failed += u64::from(stats.witnesses_failed);
        let js_alloc_json = result.as_ref().ok().map(|lowered| {
            crate::analysis::js_alloc::JsAllocSites::of_with_origins(
                &lowered.decl,
                &lowered.js_alloc_origins,
            )
            .to_json()
        });
        if !table.contains_key(&name)
            && result.is_ok()
            && (repr.clones.contains_key(&name) || abi_clones.contains_key(&name))
        {
            if let Some(clone) = work_table.get(&name).cloned() {
                table.insert(name.clone(), clone);
            }
        }
        let Some(item) = table.get_mut(&name) else {
            continue;
        };
        let mut outcome = LaneOutcome {
            class: item
                .jir
                .summary
                .as_ref()
                .map(|s| s.class)
                .unwrap_or_default(),
            name,
            lane: ItemLane::Fallback,
            decline: None,
            refs_equal: true,
            bytes_baseline: item.js.len(),
            bytes_jir: 0,
            bytes_min: 0,
            alloc_json: alloc,
            js_alloc_json,
            host_args: stats.host_args.clone(),
            slice_iterator_would_lower: stats.slice_iterator_loops_would_lower,
            slice_iterator_final_emitted: 0,
        };
        match result {
            Err(decline) => outcome.decline = Some(decline),
            Ok(lowered) => match apply(dcx, item, lowered, &defined, &live) {
                Ok(applied) => {
                    outcome.lane = ItemLane::Jir;
                    outcome.bytes_jir = applied.bytes;
                    outcome.bytes_min = applied.bytes_min;
                    outcome.refs_equal = !applied.refs_changed;
                    // What `js-opt-report`'s `passes.applied` reads: a pass is applied to an item
                    // only when it changed the item AND that item's lowering is what was printed.
                    // Under `js-opt=report` the change was made to a clone and dropped, so it
                    // counts as `would_apply` and nothing else.
                    if stats.sroa_changed {
                        passes.would_apply.sroa += 1;
                        if ctx.opts.applies(OptimizerPass::Sroa) {
                            passes.applied.sroa += 1;
                            item.optimization.applied.insert("sroa".to_owned());
                        }
                    }
                    if stats.ast_dce_changed || o_stats.dce_items.contains(&outcome.name) {
                        passes.would_apply.ast_dce += 1;
                        if ctx.opts.applies(OptimizerPass::AstDce) {
                            passes.applied.ast_dce += 1;
                            item.optimization.applied.insert("ast-dce".to_owned());
                        }
                    }
                    if ctx.opts.applies(OptimizerPass::AstDce) {
                        outcome.host_args.final_emitted = stats.host_args.would_lower;
                        outcome.slice_iterator_final_emitted =
                            stats.slice_iterator_loops_would_lower;
                    }
                    // The O group's item counts. Only the lane sidecar carries them: the report's
                    // `passes.applied` names are v1's five, and `inline`/`devirt` are not among
                    // them, so `item.optimization` is left alone.
                    if o_stats.inline.items.contains(&outcome.name) {
                        passes.would_apply.inline += 1;
                        if ctx.opts.applies(OptimizerPass::AstDce) {
                            passes.applied.inline += 1;
                        }
                    }
                    if o_stats.devirt.items.contains(&outcome.name) {
                        passes.would_apply.devirt += 1;
                        if ctx.opts.applies(OptimizerPass::AstDce) {
                            passes.applied.devirt += 1;
                        }
                    }
                }
                Err(decline) => {
                    outcome.refs_equal = false;
                    outcome.decline = Some(decline);
                }
            },
        }
        outcomes.push(outcome);
    }

    // `js-str=bytes` is a JIR-lane representation: an item that keeps the baseline's text holds
    // its `str` values as UTF-16 text, and the program would mix the two. Refused, never mixed.
    if crate::lower::bstr::enabled() {
        for outcome in outcomes.iter().filter(|o| o.lane == ItemLane::Fallback) {
            dcx.err(format!(
                "rustc_codegen_js: `js-str=bytes` needs every item on the JIR lane, but `{}` fell \
                 back to the baseline text ({:?})",
                outcome.name.as_str(),
                outcome.decline
            ));
        }
    }

    let referenced: BTreeSet<JsName> = table
        .values()
        .flat_map(|item| item.refs.iter().cloned())
        .collect();
    let existing: BTreeSet<JsName> = table.keys().cloned().collect();
    let helper_items = crate::lower::js::i64_pair::needed_helper_items(&referenced, &existing);
    let added_items = added_items || !helper_items.is_empty();
    for helper in helper_items {
        if let std::collections::btree_map::Entry::Vacant(slot) = table.entry(helper.name.clone()) {
            slot.insert(helper);
        }
    }

    LaneRun {
        outcomes,
        passes,
        o: o_stats,
        range_asserts,
        range_comparisons,
        witnesses_recorded,
        witnesses_failed,
        abi_json: if abi_applied {
            plan.json(true)
        } else {
            reported
        },
        physical_json: Some(physical.json()),
        async_json: Some(async_plan.json()),
        // The representation plan's accounting (Phase 4, D12): present whenever its analysis
        // ran, filled by the solver in report and on alike.
        repr: repr_ran.then(|| repr.block.clone()),
        plan_retries,
        added_items,
        byte_views,
        native_vec_enabled: ctx.heap.native_vec_enabled,
    }
}

/// Empties provably no-op `DropGlue(String)` and `DropGlue(Vec<T, Global>)` JIR `Func` bodies
/// (plus rustc-elaborated `.buf` / no-op `OpKind::Drop`s and transitive pure-drop-only
/// `DropGlue`s) right before post-repr `o_group_with_mode`, so `prune_invoke`, `witness`,
/// `inline`, and `dce` eliminate dead `try/catch` unwind landing pads automatically.
fn prune_noop_heap_drop_glues(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    heap: &crate::analysis::heap::HeapPlan,
) {
    use rcgjs_object::jir::repr::{OpKind, RBlock, RBlockId, RTerm};
    use rcgjs_object::jir::semantic::{BodySemantic, DropGlue};

    let nstr = heap.native_string_enabled;
    let nvec = heap.native_vec_enabled && !heap.non_rc_raw_alloc_reachable;
    if !nstr && !nvec {
        return;
    }

    let empty_blocks = |func: &Func| {
        vec![RBlock {
            id: RBlockId(0),
            params: func
                .blocks
                .first()
                .map(|b| b.params.clone())
                .unwrap_or_default(),
            ops: Vec::new(),
            term: RTerm::Return(None),
        }]
    };

    let mut noop_funcs: BTreeSet<JsName> = BTreeSet::new();
    for (name, res) in funcs.iter_mut() {
        let Ok(func) = res else { continue };
        let item = table.get(name);
        let is_noop_str = nstr && crate::analysis::heap::is_string_drop_glue(item);
        let is_noop_vec =
            nvec && crate::analysis::heap::is_noop_global_vec_drop_glue(item, nstr, nvec);
        if is_noop_str || is_noop_vec {
            func.blocks = empty_blocks(func);
            noop_funcs.insert(name.clone());
            continue;
        }
        let Some(interns) = item.and_then(|i| i.jir.interns()) else {
            continue;
        };
        // Strip rustc-elaborated no-op `OpKind::Drop` sites (e.g., `String.vec`, `Vec.buf`,
        // `Vec<POD>`) to `DropGlue::None` so `prune_invoke` sees empty unwind chains.
        let mut to_clear: Vec<(usize, usize)> = Vec::new();
        for (bi, block) in func.blocks.iter().enumerate() {
            for (oi, op) in block.ops.iter().enumerate() {
                if let OpKind::Drop {
                    root,
                    path,
                    glue: DropGlue::Direct { .. },
                } = &op.kind
                {
                    let noop = (nstr
                        && crate::analysis::heap::drop_targets_native_string(
                            func,
                            &interns.types,
                            root,
                            path,
                        ))
                        || (nvec
                            && crate::analysis::heap::drop_targets_noop_native_vec(
                                func,
                                &interns.types,
                                interns,
                                root,
                                path,
                                nstr,
                                nvec,
                            ));
                    if noop {
                        to_clear.push((bi, oi));
                    }
                }
            }
        }
        for (bi, oi) in to_clear {
            if let OpKind::Drop { glue, .. } = &mut func.blocks[bi].ops[oi].kind {
                *glue = DropGlue::None;
            }
        }
        if func.blocks.len() == 1
            && func.blocks[0].ops.is_empty()
            && matches!(func.blocks[0].term, RTerm::Return(None))
        {
            noop_funcs.insert(name.clone());
        }
    }

    // Fixpoint: collapse composite `DropGlue`s (e.g. `Option<String>`, `struct S { v: Vec<u32> }`)
    // whose only remaining operations are pure projections and drops targeting `noop_funcs`.
    for _ in 0..4 {
        let mut newly_empty: Vec<JsName> = Vec::new();
        for (name, res) in funcs.iter() {
            if noop_funcs.contains(name) {
                continue;
            }
            let Ok(func) = res else { continue };
            let Some(item) = table.get(name) else {
                continue;
            };
            if !matches!(
                item.jir.body.as_ref().map(|b| b.semantic),
                Some(BodySemantic::DropGlue(_))
            ) {
                continue;
            }
            let Some(interns) = item.jir.interns() else {
                continue;
            };
            let is_pure_noop = func.blocks.iter().all(|b| {
                let ops_ok = b.ops.iter().all(|op| match &op.kind {
                    OpKind::Copy(_)
                    | OpKind::Move(_)
                    | OpKind::Const(_)
                    | OpKind::Discriminant(_)
                    | OpKind::Field { .. }
                    | OpKind::BorrowView { .. }
                    | OpKind::RawView { .. }
                    | OpKind::CellLoad(_)
                    | OpKind::Unary { .. } => true,
                    OpKind::Binary { op, .. } => !matches!(
                        op,
                        rcgjs_object::jir::semantic::BinaryOp::Div
                            | rcgjs_object::jir::semantic::BinaryOp::Rem
                    ),
                    OpKind::Drop { glue, .. } => match glue {
                        DropGlue::None => true,
                        DropGlue::Direct { link_name, .. } => interns
                            .str(*link_name)
                            .is_some_and(|s| noop_funcs.contains(&JsName::new(s.to_owned()))),
                        DropGlue::Dynamic => false,
                    },
                    _ => false,
                });
                let term_ok = matches!(
                    b.term,
                    RTerm::Return(None)
                        | RTerm::Goto(_, _)
                        | RTerm::Switch { .. }
                        | RTerm::Invoke { .. }
                        | RTerm::Throw(None)
                );
                ops_ok && term_ok
            });
            if is_pure_noop {
                newly_empty.push(name.clone());
            }
        }
        if newly_empty.is_empty() {
            break;
        }
        for name in newly_empty {
            if let Some(Ok(func)) = funcs.get_mut(&name) {
                func.blocks = empty_blocks(func);
            }
            noop_funcs.insert(name);
        }
    }
}

/// What every lowering of one lane pass shares: the options and the three program-wide indexes.
struct Ctx<'a> {
    opts: &'a crate::opts::Opts,
    items: &'a ItemIndex,
    drop_glue: &'a DropGlueIndex,
    vtables: &'a VtableIndex,
    byte_views: crate::analysis::byte_views::ByteViewPlan,
    heap: crate::analysis::heap::HeapPlan,
}

/// The names the lane offers to the lowerer, sorted and deduplicated.
///
/// `reachable` is a walk order; the sidecar and the binary search over its rows are by name. An
/// item is offered when it carries a decoded body, its class is promoted, it is not on the allow
/// list, and `js-lane-only` (when given) names it — see the file header.
fn offered(
    table: &BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    opts: &crate::opts::Opts,
) -> Vec<JsName> {
    let allowed = allowlist();
    let only: BTreeSet<&str> = opts.lane_only.iter().map(String::as_str).collect();
    let mut names: Vec<JsName> = reachable
        .iter()
        .filter(|name| {
            let Some(item) = table.get(*name) else {
                return false;
            };
            // A body the link step decoded is the whole of what the lowerer can be given: an item
            // without one was never encoded (it is synthesized) and is not a lane decision at all.
            let (Some(_), Some(summary), Some(_)) = (
                item.jir.body.as_ref(),
                item.jir.summary.as_ref(),
                item.jir.interns(),
            ) else {
                return false;
            };
            opts.lane_classes.contains(summary.class)
                && !allowed.contains(name.as_str())
                && (only.is_empty() || only.contains(name.as_str()))
        })
        .cloned()
        .collect();
    names.sort();
    names.dedup();
    names
}

/// One item's declaration, or the decline that stopped it, what the passes did to it, and its
/// alloc-site accounting as the sidecar's `alloc` block (`None` for an item with no `Func`).
type Lowered = (
    JsName,
    Result<LoweredItem, Decline>,
    OptStats,
    Option<String>,
);

/// The lowerer's view of one item, or `None` when the item is not a lane decision.
fn lower_input<'a>(
    name: &'a JsName,
    item: &'a LinkItem,
    ctx: &'a Ctx<'a>,
    spills: &'a [JsName],
) -> Option<LowerInput<'a>> {
    lower_input_with_retargets(name, item, ctx, spills, crate::lower::EMPTY_CALL_RETARGETS)
}

fn lower_input_with_retargets<'a>(
    name: &'a JsName,
    item: &'a LinkItem,
    ctx: &'a Ctx<'a>,
    spills: &'a [JsName],
    call_retargets: &'a crate::lower::CallRetargets,
) -> Option<LowerInput<'a>> {
    let interns = item.jir.interns()?;
    Some(LowerInput {
        name,
        summary: item.jir.summary.as_ref()?,
        body: item.jir.body.as_ref()?,
        origins: item.jir.origins.as_ref(),
        interns,
        table: &interns.types,
        opts: ctx.opts,
        items: ctx.items,
        drop_glue: ctx.drop_glue,
        vtables: ctx.vtables,
        spills,
        abi: None,
        physical: None,
        call_retargets,
        bigints: None,
        byte_views: ctx.byte_views,
        heap: Some(&ctx.heap),
        nested_clone: crate::lower::NestedClone::default(),
    })
}

/// Builds the exact compact program for one physical ABI label vector.
///
/// `reuse` hands back the lowering of every function the trial provably leaves at its neutral
/// input (`plan::lowering_reuse`); everything after the per-item lowering runs over the whole
/// program.
fn physical_program_metrics(
    source_table: &BTreeMap<JsName, LinkItem>,
    source_funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    abi: &AbiPlan,
    ctx: &Ctx<'_>,
    nested_clone: &BTreeMap<JsName, crate::lower::NestedClone>,
    reuse: &mut crate::plan::lowering_reuse::Reuse,
) -> Option<crate::plan::physical_cost::ProgramMetrics> {
    crate::profile::bump(crate::profile::Counter::TrialPhysical);
    reuse.begin_trial(source_table, source_funcs, ctx.opts);
    let mut opts = ctx.opts.clone();
    opts.optimizer = crate::opts::OptimizerMode::On;
    opts.minify = true;
    opts.minify_items = true;
    let measured_ctx = Ctx {
        opts: &opts,
        items: ctx.items,
        drop_glue: ctx.drop_glue,
        vtables: ctx.vtables,
        byte_views: ctx.byte_views,
        heap: ctx.heap.clone(),
    };
    let mut table = source_table.clone();
    let mut funcs = source_funcs.clone();
    abi.install_clones(&mut table, &mut funcs);
    abi.rewrite(&mut funcs, &table);

    let mut lowered = Vec::new();
    let mut allocation_terms: BTreeMap<JsName, (u64, u64, u64)> = BTreeMap::new();
    let mut direct_calls: BTreeMap<JsName, BTreeSet<JsName>> = BTreeMap::new();
    let mut loop_calls: BTreeMap<JsName, BTreeSet<JsName>> = BTreeMap::new();
    for (name, entry) in &funcs {
        let Ok(func) = entry else {
            if abi.touches(name) {
                return None;
            }
            continue;
        };
        let item = table.get(name)?;
        let mut input = lower_input_with_retargets(
            name,
            item,
            &measured_ctx,
            abi.spill_names(),
            abi.call_retargets(name),
        )?;
        // Slice, tagged, and recursive lanes expand in the JavaScript lowerer rather than the
        // structural JIR rewrite. Exact economics must therefore lower the trial with the trial
        // ABI, just as the final phase-C pass does, or those candidates price as byte-identical
        // to the baseline and can never be selected.
        input.abi = Some(abi);
        input.nested_clone = nested_clone
            .get(name)
            .or_else(|| abi.clones().get(name).and_then(|s| nested_clone.get(s)))
            .copied()
            .unwrap_or_default();
        match reuse.lower_physical(abi, name, &input, func) {
            Ok(lowering) => {
                allocation_terms.insert(name.clone(), lowering.alloc);
                direct_calls.insert(name.clone(), lowering.direct_calls);
                loop_calls.insert(name.clone(), lowering.loop_calls);
                lowered.push((name.clone(), lowering.js, lowering.refs));
            }
            Err(_) if abi.touches(name) => return None,
            Err(_) => {}
        }
    }

    for (name, js, refs) in lowered {
        let item = table.get_mut(&name)?;
        item.js = js;
        item.refs = refs;
    }
    for spill in abi.spill_items() {
        table.insert(spill.name.clone(), spill);
    }
    let referenced: BTreeSet<JsName> = table
        .values()
        .flat_map(|item| item.refs.iter().cloned())
        .collect();
    let existing: BTreeSet<JsName> = table.keys().cloned().collect();
    for helper in crate::lower::js::i64_pair::needed_helper_items(&referenced, &existing) {
        if let std::collections::btree_map::Entry::Vacant(slot) = table.entry(helper.name.clone()) {
            slot.insert(helper);
        }
    }

    let mut reachable = crate::plan::representation::measurement_reach(&table, &opts.roots);
    if opts.analyzes(OptimizerPass::LinkDedup)
        && crate::link_dedup::merge_duplicates_for_measurement(
            &mut table,
            &reachable,
            opts.source_map,
        )
    {
        reachable = crate::plan::representation::measurement_reach(&table, &opts.roots);
    }
    let stubs = crate::link::stub_missing(&table, &reachable);
    let mut items = crate::link::order(&table, &reachable);
    items.extend(stubs.iter());
    let (program, _) = write_program(&prologue("physical-cost"), &items);
    let renames = crate::minify::item_renames(&program, &items);
    let mut program = crate::minify::apply_renames(&program, &renames);
    let mut direct = crate::plan::cost::runtime_members(&program);
    if ctx.byte_views.raw_byte_derefs && !direct.is_empty() {
        direct.insert("_bv_enable".to_owned());
    }
    program.push_str(&crate::plan::cost::helper_program(&direct));

    // An allocation in a directly called callee is still loop-carried when the final JavaScript
    // invokes that callee from a structured loop. Propagate the lexical loop edge through the
    // final direct-call graph, then count each allocation site once: every site in a hot callee,
    // otherwise only sites lexically nested in that item's own loops.
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
            (
                allocations.saturating_add(item_allocations),
                loop_allocations.saturating_add(if hot_items.contains(name) {
                    item_allocations
                } else {
                    item_loops
                }),
                alloc_term.saturating_add(propagated_alloc_term(
                    item_allocations,
                    item_term,
                    hot_items.contains(name),
                    opts.weight_alloc,
                )),
            )
        },
    );
    Some(crate::plan::physical_cost::program_metrics(
        &program,
        allocations,
        loop_allocations,
        alloc_term,
    ))
}

/// Charges every allocation in a transitively loop-called item at depth one or deeper.
///
/// The declaration-local term already carries exact higher lexical depths, so propagation is a
/// lower bound rather than a replacement. Keeping this calculation beside the final direct-call
/// closure makes `loop_allocations` and the balanced profile's score describe the same emitted
/// program.
fn propagated_alloc_term(
    allocations: u64,
    lexical_term: u64,
    transitively_hot: bool,
    weight_alloc: u64,
) -> u64 {
    if transitively_hot {
        lexical_term.max(allocations.saturating_mul(weight_alloc).saturating_mul(4))
    } else {
        lexical_term
    }
}

/// Phase A: the JIR-R of every offered **function**, or the decline that stopped it.
///
/// A `static` and a vtable are absent: they have no `Func`, and phase C lowers them from the
/// body's own tree.
fn build_funcs(
    table: &BTreeMap<JsName, LinkItem>,
    names: &[JsName],
    ctx: &Ctx<'_>,
) -> BTreeMap<JsName, Result<Func, Decline>> {
    let mut funcs = BTreeMap::new();
    for name in names {
        let Some(item) = table.get(name) else {
            continue;
        };
        // Phase A never prints, so the register names it would print are not needed here.
        let Some(input) = lower_input(name, item, ctx, &[]) else {
            continue;
        };
        if !matches!(input.body.kind, crate::jir::BodyKind::Function) {
            continue;
        }
        funcs.insert(name.clone(), crate::lower::build_func(&input));
    }
    funcs
}

/// Phase O (phase 5, D5): the interprocedural group over the whole `funcs` map.
///
/// Summaries first ([`crate::analysis::effects`]), then the inliner and the devirtualizer, then
/// `summary_check::enforce` demotes what the rewrites staled, then one dce run per func with the
/// pure-call oracle the surviving summaries answer (D7c). Every rewrite is verified and witness-
/// checked; a failure reverts that item's rewrite and the link goes on. `RCGJS_JIR_PASSES`
/// selects the group's members exactly as it does the per-item passes.
fn o_group(
    table: &mut BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ctx: &Ctx<'_>,
    live: &BTreeSet<JsName>,
) -> OGroup {
    o_group_with_mode(table, funcs, ctx, live, false, None)
}

fn o_group_with_mode(
    table: &mut BTreeMap<JsName, LinkItem>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    ctx: &Ctx<'_>,
    live: &BTreeSet<JsName>,
    allow_std_post_repr: bool,
    call_retargets: Option<&BTreeMap<JsName, crate::lower::CallRetargets>>,
) -> OGroup {
    use crate::analysis::{callgraph, effects, summary_check};
    let mut group = OGroup::default();
    let run_inline = crate::opt::o_pass_enabled("inline");
    let run_devirt = crate::opt::o_pass_enabled("devirt");
    let run_dce = crate::opt::o_pass_enabled("dce");
    if !(run_inline || run_devirt || run_dce) {
        return group;
    }
    if run_dce {
        let pruned = crate::opt::prune_invoke::run_checked(table, funcs, live);
        group.witnesses_recorded += pruned.witnesses_recorded;
        group.witnesses_failed += pruned.witnesses_failed;
    }
    let interns_map: BTreeMap<JsName, &crate::jir::Interns> = table
        .iter()
        .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
        .collect();
    if allow_std_post_repr
        && run_devirt
        && matches!(ctx.opts.profile, Some(crate::opts::Profile::ReleaseSpeed))
    {
        let pre_devirt = crate::opt::devirtualize::run(funcs, &interns_map, table, ctx.vtables);
        group.devirt.applied += pre_devirt.applied;
        group.devirt.witnesses_recorded += pre_devirt.witnesses_recorded;
        group.devirt.witnesses_failed += pre_devirt.witnesses_failed;
        group.devirt.items.extend(pre_devirt.items);
    }
    let mut graph = callgraph::build(funcs, &interns_map);
    let mut summaries = effects::compute(funcs, &interns_map, &graph);
    group.summaries_items = summaries.len() as u32;

    // Use the same reserved/public-name policy as final emission. One deterministic name map
    // is shared by every probe in this group, so symbol length cannot masquerade as a body win.
    let items: Vec<_> = live.iter().filter_map(|name| table.get(name)).collect();
    let (program, _) = write_program("", &items);
    let inline_renames = crate::minify::item_renames(&program, &items);

    let empty_retargets = crate::lower::CallRetargets::new();
    // One item priced through the real phase-C pipeline on a clone, exactly the solver's rule.
    let measure_full = |name: &JsName, func: &Func| -> Option<(u64, BTreeSet<JsName>)> {
        let item = table.get(name)?;
        let retargets = call_retargets
            .and_then(|map| map.get(name))
            .unwrap_or(&empty_retargets);
        let input = lower_input_with_retargets(name, item, ctx, &[], retargets)?;
        crate::plan::cost::measure_inline_item(&input, func, &inline_renames)
    };

    if run_inline {
        // The measured program: the +0.5% budget's denominator and the per-item entry sizes. A
        // func the probe cannot price contributes its baseline text bytes to the total only.
        let mut sizes: BTreeMap<JsName, u64> = BTreeMap::new();
        let mut measured_refs: BTreeMap<JsName, BTreeSet<JsName>> = BTreeMap::new();
        let mut program: u64 = 0;
        for (name, func) in funcs.iter() {
            match func.as_ref().ok().and_then(|func| measure_full(name, func)) {
                Some((bytes, refs)) => {
                    sizes.insert(name.clone(), bytes);
                    measured_refs.insert(name.clone(), refs);
                    program += bytes;
                }
                None => program += table.get(name).map_or(0, |item| item.js.len() as u64),
            }
        }
        // Recompute live reachability and reference counts from post-O measured refs so the
        // sole-referrer oracle does not stay anchored to stale pre-opt `item.refs`.
        let effective_live: BTreeSet<JsName> = {
            let mut reach = BTreeSet::new();
            let mut work: Vec<JsName> = table
                .iter()
                .filter(|(name, item)| {
                    live.contains(*name)
                        && (item.linkage.root || ctx.opts.roots.iter().any(|r| r == name.as_str()))
                })
                .map(|(name, _)| name.clone())
                .collect();
            while let Some(cur) = work.pop() {
                if !live.contains(&cur) || !reach.insert(cur.clone()) {
                    continue;
                }
                if let Some(refs) = measured_refs.get(&cur) {
                    work.extend(refs.iter().cloned());
                } else if let Some(item) = table.get(&cur) {
                    work.extend(item.refs.iter().cloned());
                }
            }
            reach
        };
        // The live reference counts: the single-caller criterion's oracle.
        let mut referrers: BTreeMap<JsName, (u32, Option<JsName>)> = BTreeMap::new();
        for (name, item) in table.iter() {
            if !effective_live.contains(name) {
                continue;
            }
            let refs_set = measured_refs.get(name).unwrap_or(&item.refs);
            for target in refs_set {
                if target == name {
                    continue;
                }
                let entry = referrers.entry(target.clone()).or_insert((0, None));
                entry.0 += 1;
                entry.1 = Some(name.clone());
            }
        }
        let sole = |callee: &JsName, caller: &JsName| -> bool {
            table.get(callee).is_some_and(|item| !item.linkage.root)
                && !ctx.opts.roots.iter().any(|root| root == callee.as_str())
                && referrers
                    .get(callee)
                    .is_some_and(|(count, who)| *count == 1 && who.as_ref() == Some(caller))
        };
        let zombie_free =
            |name: &JsName| table.get(name).is_some_and(|item| item.zombies.is_empty());
        let mut metadata: BTreeMap<_, _> = table
            .iter()
            .filter_map(|(name, item)| {
                let interns = item.jir.interns()?;
                Some((
                    name.clone(),
                    crate::opt::inline::Metadata {
                        universe: interns,
                        interns: std::borrow::Cow::Borrowed(interns),
                        origins: item.jir.origins.as_ref().map(std::borrow::Cow::Borrowed),
                        header: item.jir.header(),
                        summary: item.jir.summary.as_ref().map(std::borrow::Cow::Borrowed),
                    },
                ))
            })
            .collect();
        let measure_imported =
            |name: &JsName, func: &Func, metadata: &crate::opt::inline::Metadata<'_>| {
                let item = table.get(name)?;
                let retargets = call_retargets
                    .and_then(|map| map.get(name))
                    .unwrap_or(&empty_retargets);
                let mut input = lower_input_with_retargets(name, item, ctx, &[], retargets)?;
                input.interns = &metadata.interns;
                input.table = &metadata.interns.types;
                input.origins = metadata.origins.as_deref();
                if let Some(summary) = metadata.summary.as_deref() {
                    input.summary = summary;
                }
                crate::plan::cost::measure_inline_item(&input, func, &inline_renames)
            };
        group.inline = crate::opt::inline::run_with_profile_and_retargets(
            funcs,
            &mut metadata,
            &graph,
            ctx.items,
            &measure_imported,
            &sole,
            &zombie_free,
            program,
            &sizes,
            allow_std_post_repr,
            ctx.opts.profile,
            call_retargets,
        );
        let updates: Vec<_> = metadata
            .into_iter()
            .filter_map(|(name, metadata)| {
                let interns_owned = matches!(metadata.interns, std::borrow::Cow::Owned(_));
                let summary_owned = matches!(metadata.summary, Some(std::borrow::Cow::Owned(_)));
                if !interns_owned && !summary_owned {
                    return None;
                }
                Some((
                    name,
                    interns_owned.then(|| metadata.interns.into_owned()),
                    interns_owned.then(|| metadata.origins.map(|o| o.into_owned())),
                    summary_owned.then(|| metadata.summary.map(|s| s.into_owned())),
                ))
            })
            .collect();
        drop(interns_map);
        for (name, interns, origins, summary) in updates {
            let item = table
                .get_mut(&name)
                .expect("inlined caller remains in the item table");
            if let Some(interns) = interns {
                item.jir.use_interns(interns);
                item.jir.origins = origins.flatten();
            }
            if let Some(Some(summary)) = summary {
                item.jir.summary = Some(summary);
            }
        }
    }
    let pruned_post_inline = if run_dce && group.inline.applied > 0 {
        let pruned = crate::opt::prune_invoke::run_checked(table, funcs, live);
        group.witnesses_recorded += pruned.witnesses_recorded;
        group.witnesses_failed += pruned.witnesses_failed;
        pruned.modified > 0
    } else {
        false
    };
    let interns_map: BTreeMap<JsName, &crate::jir::Interns> = table
        .iter()
        .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
        .collect();
    if pruned_post_inline {
        graph = callgraph::build(funcs, &interns_map);
        summaries = effects::compute(funcs, &interns_map, &graph);
        group.summaries_items = summaries.len() as u32;
    }
    if run_devirt {
        let post_devirt = crate::opt::devirtualize::run(funcs, &interns_map, table, ctx.vtables);
        group.devirt.applied += post_devirt.applied;
        group.devirt.witnesses_recorded += post_devirt.witnesses_recorded;
        group.devirt.witnesses_failed += post_devirt.witnesses_failed;
        group.devirt.items.extend(post_devirt.items);
    }
    if run_dce {
        let projection = crate::opt::projection::run(funcs, &interns_map);
        group.dce_items.extend(projection.items);
        group.witnesses_recorded += projection.witnesses_recorded;
        group.witnesses_failed += projection.witnesses_failed;
    }
    // D1: revalidate against the rewritten bodies; a claim the recompute cannot confirm demotes
    // to top, and everything that then reads the map sees `op.missing-summary`-shaped absence.
    if group.inline.applied > 0 || group.devirt.applied > 0 {
        graph = callgraph::build(funcs, &interns_map);
    }
    let violations = summary_check::enforce(&mut summaries, funcs, &interns_map, &graph);
    group.summaries_validated = group
        .summaries_items
        .saturating_sub(violations.len() as u32);
    group.summaries_top = summaries.iter().filter(|(_, s)| s.is_top()).count() as u32;

    if run_dce {
        // D7(c): the whole-map dce run whose pure-call oracle the summaries answer — a call to a
        // `Const`/`Read`, non-unwinding callee with an unused result is deletable.
        let names: Vec<JsName> = funcs.keys().cloned().collect();
        for name in names {
            let Some(interns) = interns_map.get(&name).copied() else {
                continue;
            };
            let Some(Ok(func)) = funcs.get(&name) else {
                continue;
            };
            let oracle = |callee: &crate::jir::semantic::Callee| match callee {
                crate::jir::semantic::Callee::Direct { link_name, .. } => {
                    // An item with recorded zombies owes its compile-time diagnostics: deleting
                    // the call would drop the item and silence them, purity notwithstanding.
                    let diagnosed = interns
                        .str(*link_name)
                        .map(|name| JsName::new(name.to_owned()))
                        .and_then(|name| table.get(&name))
                        .is_some_and(|item| !item.zombies.is_empty());
                    !diagnosed
                        && summaries
                            .callee(interns, *link_name)
                            .is_some_and(|summary| {
                                summary.purity != effects::Purity::Effect && !summary.may_unwind
                            })
                }
                _ => false,
            };
            let options = crate::opt::dce::Options {
                no_constant_params: !crate::opt::o_pass_enabled("constprop"),
                no_deletions: false,
                pure_call: Some(&oracle),
            };
            let mut rewritten = func.clone();
            let mut witnesses = Vec::new();
            if !crate::opt::dce::run_with(&mut rewritten, &interns.types, &options, &mut witnesses)
            {
                continue;
            }
            group.witnesses_recorded += witnesses.len() as u32;
            let verified = rewritten
                .verify_carriers(&interns.types, crate::opt::observed_carriers(&rewritten))
                .is_ok();
            let checked = !crate::opt::witness::enabled()
                || crate::opt::witness::check_all(&witnesses, &rewritten, &interns.types).is_ok();
            if !verified || !checked {
                if verified {
                    group.witnesses_failed += 1;
                }
                continue;
            }
            group.dce_items.insert(name.clone());
            funcs.insert(name, Ok(rewritten));
        }
        // Deletions can stale a summary too; the group ends validated (D1: enforce after every
        // pass group). Nothing downstream consumes the map — this keeps the sidecar honest.
        let violations = summary_check::enforce(&mut summaries, funcs, &interns_map, &graph);
        group.summaries_validated = group
            .summaries_validated
            .saturating_sub(violations.len() as u32);
        group.summaries_top = summaries.iter().filter(|(_, s)| s.is_top()).count() as u32;
    }
    group.nonthrowing_items = summaries
        .iter()
        .filter(|(_, s)| !s.may_unwind)
        .map(|(name, _)| name.clone())
        .collect();
    // The `RCGJS_REPR_DEBUG_FUNC` precedent, for the group's rewrites.
    if let Ok(filter) = std::env::var("RCGJS_O_DEBUG_FUNC") {
        for (name, func) in funcs.iter() {
            if name.as_str().contains(&filter) {
                if let Ok(func) = func {
                    eprintln!("o-func: {} = {func:#?}", name.as_str());
                }
            }
        }
    }
    group
}

type ReportProbe<'a> = (
    &'a BTreeMap<JsName, LinkItem>,
    &'a BTreeMap<JsName, Result<Func, Decline>>,
);

/// Phase C: optimize and print each item. Returns the declarations and whether the run must retry.
///
/// Nothing here touches the table: a retry has to be decided before any item is replaced.
#[allow(clippy::too_many_arguments)] // One call site; the arguments are the driver's phases.
fn prepare_opt_func(
    name: &JsName,
    func: &mut Func,
    table_types: &crate::jir::types::TypeTable,
    interns: &crate::jir::Interns,
    opts: &crate::opts::Opts,
    report_probe: Option<ReportProbe<'_>>,
) -> Result<(OptStats, Option<String>), Decline> {
    let mut stats = OptStats::default();
    let probe = if let Some((probe_table, probe_funcs)) = report_probe {
        if let (Some(probe_item), Some(Ok(probe_func))) =
            (probe_table.get(name), probe_funcs.get(name))
        {
            if let Some(probe_interns) = probe_item.jir.interns() {
                let mut probe = probe_func.clone();
                if crate::opt::optimize(
                    &mut probe,
                    &probe_interns.types,
                    probe_interns,
                    opts,
                    &mut stats,
                )
                .is_err()
                {
                    stats = OptStats::default();
                }
            }
        }
        Ok(())
    } else {
        crate::opt::optimize(func, table_types, interns, opts, &mut stats)
    };
    match probe {
        Err(decline) => {
            if std::env::var("RCGJS_OPT_DEBUG_FUNC")
                .is_ok_and(|filter| name.as_str().contains(&filter))
            {
                eprintln!("opt-func: {} declined: {decline}", name.as_str());
            }
            Err(decline)
        }
        Ok(()) => {
            // The `RCGJS_O_DEBUG_FUNC` precedent, after the per-item passes: the
            // exact `Func` the alloc analysis counts and the printer prints.
            if std::env::var("RCGJS_OPT_DEBUG_FUNC")
                .is_ok_and(|filter| name.as_str().contains(&filter))
            {
                eprintln!("opt-func: {} = {func:#?}", name.as_str());
            }
            // Over the final `Func`, so what is counted is what is printed.
            // Counted in every optimizer mode: `js-opt=report` rewrites nothing
            // and this is what it has to report.
            let alloc = crate::analysis::alloc_sites::AllocSites::of(func, interns).to_json();
            Ok((stats, Some(alloc)))
        }
    }
}

#[allow(clippy::too_many_arguments)] // One call site; the arguments are the driver's phase outputs.
fn lower_all(
    table: &BTreeMap<JsName, LinkItem>,
    names: &[JsName],
    ctx: &Ctx<'_>,
    mut funcs: BTreeMap<JsName, Result<Func, Decline>>,
    report_probe: Option<ReportProbe<'_>>,
    plan: &AbiPlan,
    repr: &crate::plan::representation::ReprOutcome,
    physical: &crate::plan::physical::PhysicalPlan,
    defined: &BTreeSet<JsName>,
    live: &BTreeSet<JsName>,
    fault: Option<&str>,
    bigints: Option<&crate::lower::bigint::BigIntNames>,
) -> (Vec<Lowered>, bool) {
    let mut prepared_meta: BTreeMap<JsName, (OptStats, Option<String>)> = BTreeMap::new();
    let ascii = if crate::lower::bstr::enabled() {
        for name in names {
            if repr.bodies.contains_key(name) {
                continue;
            }
            let Some(item) = table.get(name) else {
                continue;
            };
            if item.jir.summary.is_none() || item.jir.body.is_none() {
                continue;
            }
            let Some(interns) = item.jir.interns() else {
                continue;
            };
            let Some(entry) = funcs.get_mut(name) else {
                continue;
            };
            let Ok(func) = entry else {
                continue;
            };
            match prepare_opt_func(name, func, &interns.types, interns, ctx.opts, report_probe) {
                Ok(meta) => {
                    prepared_meta.insert(name.clone(), meta);
                }
                Err(decline) => {
                    *entry = Err(decline);
                }
            }
        }
        let live_funcs: BTreeMap<JsName, &Func> = funcs
            .iter()
            .filter_map(|(name, res)| {
                if repr.bodies.contains_key(name) {
                    return None;
                }
                res.as_ref().ok().map(|f| (name.clone(), f))
            })
            .collect();
        let roots: BTreeSet<JsName> = ctx
            .opts
            .roots
            .iter()
            .map(|r| JsName::new(r.clone()))
            .collect();
        Some(crate::analysis::ascii::compute(
            &live_funcs,
            table,
            repr,
            plan,
            &roots,
            defined,
            live,
        ))
    } else {
        None
    };

    let mut lowered = Vec::with_capacity(names.len());
    let mut retry = false;
    for name in names {
        let Some(item) = table.get(name) else {
            continue;
        };
        let repr_retargets = repr
            .call_retargets
            .get(name)
            .or_else(|| {
                plan.clones()
                    .get(name)
                    .and_then(|s| repr.call_retargets.get(s))
            })
            .unwrap_or(crate::lower::EMPTY_CALL_RETARGETS);
        let abi_retargets = plan.call_retargets(name);
        let mut merged_retargets = crate::lower::CallRetargets::new();
        let retargets = if repr_retargets.is_empty() {
            abi_retargets
        } else if abi_retargets.is_empty() {
            repr_retargets
        } else {
            merged_retargets.clone_from(repr_retargets);
            for (item, target) in abi_retargets {
                let previous = merged_retargets.insert(*item, target.clone());
                debug_assert!(previous.is_none() || previous.as_ref() == Some(target));
            }
            &merged_retargets
        };
        let Some(mut input) =
            lower_input_with_retargets(name, item, ctx, plan.spill_names(), retargets)
        else {
            continue;
        };
        input.abi = Some(plan);
        input.physical = Some(physical);
        input.bigints = bigints;
        input.nested_clone = repr
            .nested_clone
            .get(name)
            .or_else(|| {
                plan.clones()
                    .get(name)
                    .and_then(|s| repr.nested_clone.get(s))
            })
            .copied()
            .unwrap_or_default();
        crate::profile::bump(crate::profile::Counter::LowerFinal);
        let mut stats = OptStats::default();
        let mut alloc = None;
        let mut result = match repr.bodies.get(name) {
            // A committed standard-operation item: the registry's native body replaces the
            // `Func` whole (Phase 4, D7), exactly like the `RCGJS_REPR_FORCE` harness.
            Some(body) => {
                funcs.remove(name);
                Ok(native_body_item(
                    name,
                    body,
                    (ctx.opts.optimizer != crate::opts::OptimizerMode::On)
                        .then_some(bigints)
                        .flatten(),
                ))
            }
            None => match funcs.remove(name) {
                // A `static` or a vtable: no `Func`, no passes, lowered from its own tree.
                None => lower_item(&input),
                Some(Err(decline)) => Err(decline),
                Some(Ok(mut func)) => {
                    let prepared = match prepared_meta.remove(name) {
                        Some(meta) => Ok(meta),
                        None => prepare_opt_func(
                            name,
                            &mut func,
                            input.table,
                            input.interns,
                            ctx.opts,
                            report_probe,
                        ),
                    };
                    match prepared {
                        Err(decline) => Err(decline),
                        Ok((opt_stats, opt_alloc)) => {
                            stats = opt_stats;
                            alloc = opt_alloc;
                            crate::lower::lower_func_with_ascii(
                                &input,
                                &func,
                                ascii.as_ref().map(|a| a.for_item(name)),
                            )
                        }
                    }
                }
            },
        };
        if let Err(decline) = &result {
            if std::env::var("RCGJS_OPT_DEBUG_FUNC")
                .is_ok_and(|filter| name.as_str().contains(&filter))
            {
                eprintln!("opt-func: {} declined: {decline}", name.as_str());
            }
        }
        let touched = plan.touches(name) || repr.touched.contains(name);
        if (touched || !physical.ready_futures.is_empty())
            && fault.is_some_and(|item| name.as_str().contains(item))
        {
            result = Err(Decline::Lower(LowerDecline::Other {
                op: OpClass::Region,
                detail: "JSC_JIR_ABI_FAULT".to_owned(),
            }));
        }
        // An item the plan changed that then declines would leave its callers calling it in the
        // shape the plan chose, which no longer exists. A lowering the driver is going to *refuse*
        // is a decline like any other, so it is asked here — before the table has been touched —
        // rather than left to the diagnostic that `apply` reports: "every caller or none" is a
        // property of the program, not of whether the link step goes on to fail.
        if touched && std::env::var("RCGJS_REPR_DEBUG_SOLVE").is_ok() {
            if let Err(decline) = &result {
                eprintln!("lane-retry: `{}` declined: {decline:?}", name.as_str());
            }
        }
        retry |= (touched || !physical.ready_futures.is_empty())
            && match &result {
                Err(_) => true,
                Ok(lowering) => refusal(
                    item,
                    lowering,
                    &refs_of(&lowering.decl, name),
                    defined,
                    live,
                )
                .is_err(),
            };
        lowered.push((name.clone(), result, stats, alloc));
    }
    (lowered, retry)
}

/// The [`LoweredItem`] of one committed standard-operation body: the registry's statements as a
/// function declaration, its BigInt constants hoisted as `js::function::finish` hoists them, its
/// refs exactly as `finish` reports them (the driver demands equality with the declaration's own
/// identifiers).
fn native_body_item(
    name: &JsName,
    body: &crate::plan::representation::NativeBody,
    bigint_names: Option<&crate::lower::bigint::BigIntNames>,
) -> crate::lower::LoweredItem {
    let mut decl = jsast::function(
        name.as_str().to_owned(),
        body.parameters.clone(),
        body.statements.clone(),
    );
    let mut bigints = BTreeSet::new();
    if let Some(names) = bigint_names {
        crate::lower::bigint::hoist(&mut decl, names, &mut bigints);
    }
    let refs = refs_of(&decl, name);
    crate::lower::LoweredItem {
        decl,
        refs,
        names: Vec::new(),
        loc: None,
        dts: None,
        zombies: Vec::new(),
        js_alloc_origins: Vec::new(),
        bigints,
    }
}

/// The item substring `JSC_JIR_ABI_FAULT` names, if any.
///
/// A **test-only** hook, and the only reason it is read in every build rather than behind
/// `debug_assertions` is that the fixtures run the release linker. What it does is make one item
/// the ABI plan rewrote (or any item while a ready driver is selected) decline in phase C,
/// which is the ordinary decline path — the whole run
/// then rebuilds with an empty plan and prints the baseline ABI. It can only ever make the link
/// step keep *less* of a plan, so a build that sets it by accident is slower, never wrong.
///
/// The retry it exists to exercise, over a fixture whose tagged-product plan is profitable:
///
/// ```text
/// JSC_JIR_ABI_FAULT=pair_option_helper_one JS_EXTRA_ARGS='js-lane=jir js-opt=on' \
///     scripts/test.sh 99_recursive_adt_iter
/// ```
///
/// which prints the fixture's own `.expected` and leaves `plan_retries: 1`, `registers: 0` and no
/// fallback in `<out>.lane.json`: the program came out in the baseline ABI, byte for byte the one
/// `js-jir-flat-abi=off` prints.
fn abi_fault() -> Option<String> {
    std::env::var("JSC_JIR_ABI_FAULT")
        .ok()
        .filter(|item| !item.is_empty())
}

/// Every item of the program by its [`ItemId`], with the JavaScript name and parameter shape a
/// host lowering needs to spell a call to it.
///
/// The index is built once per lane pass, before any item is replaced, for the same reason
/// `defined` is: the loop holds `&mut` on one item of the same table.
fn item_index(table: &BTreeMap<JsName, LinkItem>) -> (ItemIndex, DropGlueIndex) {
    let mut index = ItemIndex::new();
    let mut drop_glue = DropGlueIndex::new();
    let (_, core_crate_ids) = super::collect_panic_location_context(table);
    for (name, item) in table {
        let Some(summary) = item.jir.summary.as_ref() else {
            continue;
        };
        let Some(id) = summary.key.item else {
            continue;
        };
        let abi = summary.abi.unwrap_or_default();
        let caller_location_def = item
            .jir
            .body
            .as_ref()
            .zip(item.jir.interns())
            .and_then(|(body, interns)| super::item_caller_location_def(body, interns));
        index.insert(
            id,
            ItemEntry {
                name: name.as_str().to_owned(),
                params: abi.parameters as usize,
                abi_known: summary.abi.is_some(),
                result: item.jir.body.as_ref().map(|body| body.signature.result),
                arguments: item
                    .jir
                    .body
                    .as_ref()
                    .map(|body| body.signature.arguments.clone())
                    .unwrap_or_default(),
                caller_location: abi.caller_location,
                caller_location_def,
                is_core: core_crate_ids.contains(&id.definition.crate_id),
                used_params: BTreeSet::new(),
                standard: match item.jir.body.as_ref().map(|body| body.semantic) {
                    Some(BodySemantic::Standard(operation)) => Some(operation),
                    _ => None,
                },
                keyed_purity: None,
                keyed_read_params: None,
                keyed_may_unwind: None,
                keyed_row_capture: None,
                keyed_proxy_safe_params: None,
            },
        );
        // A body that says it IS a type's drop glue is how a host lowering names the glue of a
        // closure or a future it hands to the runtime (`lower/js/host.rs`).
        if let Some(BodySemantic::DropGlue(ty)) = item.jir.body.as_ref().map(|body| body.semantic) {
            drop_glue.insert(ty, name.as_str().to_owned());
        }
    }
    (index, drop_glue)
}

/// Records the validated effect class of direct callables used as keyed-list extractors.
fn record_keyed_purity(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    items: &mut ItemIndex,
) {
    let has_keyed_list = funcs
        .values()
        .filter_map(|func| func.as_ref().ok())
        .any(|func| {
            func.blocks.iter().flat_map(|block| &block.ops).any(|op| {
                matches!(
                    &op.kind,
                    OpKind::Call {
                        host: Some(HostPayload::KeyedList(_)),
                        ..
                    }
                )
            })
        });
    if !has_keyed_list {
        return;
    }
    let interns: BTreeMap<JsName, &crate::jir::Interns> = table
        .iter()
        .filter_map(|(name, item)| Some((name.clone(), item.jir.interns()?)))
        .collect();
    let graph = crate::analysis::callgraph::build(funcs, &interns);
    let summaries = crate::analysis::effects::compute(funcs, &interns, &graph);
    let reads = crate::analysis::effects::compute_read_origins(funcs, &interns, &graph, &summaries);
    for entry in items.values_mut() {
        let name = JsName::new(entry.name.clone());
        entry.used_params = funcs
            .get(&name)
            .and_then(|func| func.as_ref().ok())
            .map(used_parameters)
            .unwrap_or_default();
        if let Some(summary) = summaries.usable(&name) {
            entry.keyed_purity = Some(match summary.purity {
                crate::analysis::effects::Purity::Const => KeyedPurity::Const,
                crate::analysis::effects::Purity::Read => KeyedPurity::Read,
                crate::analysis::effects::Purity::Effect => KeyedPurity::Effect,
            });
            entry.keyed_read_params = reads
                .get(&name)
                .and_then(crate::analysis::effects::ReadOrigins::exact_parameters)
                .cloned();
            entry.keyed_may_unwind = Some(summary.may_unwind);
        }
    }
    record_keyed_row_capture(funcs, &interns, items);
    record_keyed_proxy_safety(funcs, &interns, items);
}

fn used_parameters(func: &Func) -> BTreeSet<u16> {
    let uses: BTreeSet<_> = func
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .ops
                .iter()
                .flat_map(|op| op.kind.uses())
                .chain(block.term.uses())
        })
        .collect();
    func.params
        .iter()
        .enumerate()
        .filter(|(_, parameter)| uses.contains(parameter))
        .filter_map(|(index, _)| u16::try_from(index).ok())
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowCaptureProof {
    Static,
    Live,
    Unknown,
}

impl RowCaptureProof {
    fn combine(self, other: RowCaptureProof) -> RowCaptureProof {
        match (self, other) {
            (RowCaptureProof::Live, _) | (_, RowCaptureProof::Live) => RowCaptureProof::Live,
            (RowCaptureProof::Unknown, _) | (_, RowCaptureProof::Unknown) => {
                RowCaptureProof::Unknown
            }
            _ => RowCaptureProof::Static,
        }
    }
}

/// Proves the first executable row lane reaches only creation-time view operations.
fn record_keyed_row_capture(
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &crate::jir::Interns>,
    items: &mut ItemIndex,
) {
    let mut local = BTreeMap::new();
    let mut edges: BTreeMap<JsName, BTreeSet<JsName>> = BTreeMap::new();
    for (name, func) in funcs {
        let Ok(func) = func else {
            continue;
        };
        let Some(item_interns) = interns.get(name).copied() else {
            local.insert(name.clone(), RowCaptureProof::Unknown);
            continue;
        };
        let mut proof = RowCaptureProof::Static;
        let callees = edges.entry(name.clone()).or_default();
        for op in func.blocks.iter().flat_map(|block| &block.ops) {
            let OpKind::Call {
                callee,
                host,
                semantic,
                args,
                ..
            } = &op.kind
            else {
                if matches!(&op.kind, OpKind::Host { .. }) {
                    proof = proof.combine(RowCaptureProof::Live);
                }
                continue;
            };
            if let Some(host) = host {
                proof = proof.combine(keyed_host_capture(host, item_interns));
                continue;
            }
            if matches!(
                semantic,
                CallSemantic::Standard(operation)
                    if crate::lower::semantic_ops::call_effects_entry(
                        *operation,
                        Some(&item_interns.types),
                        func,
                        op.result,
                        args,
                    )
                    .is_some()
            ) {
                continue;
            }
            match callee {
                Callee::Direct { link_name, .. } => match item_interns.str(*link_name) {
                    Some(target) => {
                        callees.insert(JsName::new(target.to_owned()));
                    }
                    None => proof = proof.combine(RowCaptureProof::Unknown),
                },
                _ => proof = proof.combine(RowCaptureProof::Unknown),
            }
        }
        local.insert(name.clone(), proof);
    }

    let mut proof = local.clone();
    loop {
        let before = proof.clone();
        for (name, own) in &local {
            let mut next = *own;
            for callee in edges.get(name).into_iter().flatten() {
                next = next.combine(
                    before
                        .get(callee)
                        .copied()
                        .unwrap_or(RowCaptureProof::Unknown),
                );
            }
            proof.insert(name.clone(), next);
        }
        if proof == before {
            break;
        }
    }

    for entry in items.values_mut() {
        entry.keyed_row_capture = match proof.get(&JsName::new(entry.name.clone())) {
            Some(RowCaptureProof::Static) => Some(KeyedRowCapture::Static),
            Some(RowCaptureProof::Live) => Some(KeyedRowCapture::Live),
            Some(RowCaptureProof::Unknown) | None => None,
        };
    }
}

fn keyed_host_capture(host: &HostPayload, interns: &crate::jir::Interns) -> RowCaptureProof {
    let marker = match host {
        HostPayload::ViewAbi { marker, .. } => interns.str(*marker),
        HostPayload::Marker { section, .. } => interns
            .str(*section)
            .and_then(|section| section.strip_prefix("rcgjs.tc.")),
        HostPayload::KeyedList(_) => return RowCaptureProof::Live,
        HostPayload::Extern(_) | HostPayload::Block(_) | HostPayload::Map { .. } => {
            return RowCaptureProof::Live
        }
    };
    match marker {
        Some("template" | "hole" | "content" | "text" | "list" | "push" | "hostval") => {
            RowCaptureProof::Static
        }
        Some(_) => RowCaptureProof::Live,
        None => RowCaptureProof::Unknown,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProxySafety {
    Visiting,
    Safe,
    Unsafe,
}

type ProxyPaths = BTreeSet<Vec<u32>>;
type ProxyCache = BTreeMap<(JsName, u16, ProxyPaths), ProxySafety>;

struct ProxyProgram<'a, 'i> {
    funcs: &'a BTreeMap<JsName, Result<Func, Decline>>,
    interns: &'a BTreeMap<JsName, &'i crate::jir::Interns>,
    items: &'a ItemIndex,
}

/// Proves the narrow callable-proxy ABI used for a live row parameter.
///
/// The proxy is sound while it is transported into a marker-owned closure and then consumed by
/// an operation that coerces it to a primitive. JavaScript strict equality and `switch` do not
/// coerce objects, and returning or storing the proxy would expose the representation. Those
/// shapes remain exact declines until the lowerer has a first-class live-value carrier.
fn record_keyed_proxy_safety(
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    interns: &BTreeMap<JsName, &crate::jir::Interns>,
    items: &mut ItemIndex,
) {
    let mut candidates: BTreeMap<ItemId, BTreeSet<u16>> = BTreeMap::new();
    for (name, func) in funcs {
        let Ok(func) = func else {
            continue;
        };
        let Some(item_interns) = interns.get(name).copied() else {
            continue;
        };
        for payload in func
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .filter_map(|op| match &op.kind {
                OpKind::Call {
                    host: Some(HostPayload::KeyedList(payload)),
                    ..
                } => Some(payload),
                _ => None,
            })
        {
            let Some(body) = payload.row.body else {
                continue;
            };
            let first = match item_interns
                .types
                .get(item_interns.types.peeled(payload.row.ty))
                .map(|descriptor| &descriptor.kind)
            {
                Some(TypeKind::Closure { .. }) => 1,
                Some(TypeKind::FnDef { .. }) => 0,
                _ => continue,
            };
            candidates
                .entry(body.item)
                .or_default()
                .extend([first, first + 1]);
        }
    }
    let proven = {
        let program = ProxyProgram {
            funcs,
            interns,
            items,
        };
        let mut cache = BTreeMap::new();
        let mut proven = BTreeMap::new();
        for (item, parameters) in candidates {
            let mut safe = BTreeSet::new();
            let Some(entry) = program.items.get(&item) else {
                continue;
            };
            let name = JsName::new(entry.name.clone());
            for parameter in parameters {
                if proxy_parameter_safe(&name, parameter, root_proxy_paths(), &program, &mut cache)
                {
                    safe.insert(parameter);
                }
            }
            proven.insert(item, safe);
        }
        proven
    };
    for (item, safe) in proven {
        if let Some(entry) = items.get_mut(&item) {
            entry.keyed_proxy_safe_params = Some(safe);
        }
    }
}

fn proxy_parameter_safe(
    name: &JsName,
    parameter: u16,
    seed: ProxyPaths,
    program: &ProxyProgram<'_, '_>,
    cache: &mut ProxyCache,
) -> bool {
    let key = (name.clone(), parameter, seed.clone());
    match cache.get(&key) {
        Some(ProxySafety::Safe) => return true,
        Some(ProxySafety::Visiting | ProxySafety::Unsafe) => return false,
        None => {}
    }
    cache.insert(key.clone(), ProxySafety::Visiting);
    let safe = program
        .funcs
        .get(name)
        .and_then(|func| func.as_ref().ok())
        .zip(program.interns.get(name).copied())
        .is_some_and(|(func, item_interns)| {
            proxy_body_safe(func, parameter, seed, item_interns, program, cache)
        });
    cache.insert(
        key,
        if safe {
            ProxySafety::Safe
        } else {
            ProxySafety::Unsafe
        },
    );
    safe
}

fn proxy_body_safe(
    func: &Func,
    parameter: u16,
    seed: ProxyPaths,
    item_interns: &crate::jir::Interns,
    program: &ProxyProgram<'_, '_>,
    cache: &mut ProxyCache,
) -> bool {
    let Some(initial) = func.params.get(usize::from(parameter)).copied() else {
        return false;
    };
    let mut tainted = BTreeMap::from([(initial, seed)]);
    let mut tainted_cells = BTreeMap::new();

    loop {
        let before_values = tainted.clone();
        let before_cells = tainted_cells.clone();
        for block in &func.blocks {
            for op in &block.ops {
                match &op.kind {
                    OpKind::CellStore { cell, v } => {
                        merge_proxy_paths(&mut tainted_cells, *cell, proxy_paths(&tainted, *v));
                    }
                    OpKind::SetField { base, field, v, .. } => {
                        let paths = prefixed_proxy_paths(proxy_paths(&tainted, *v), *field);
                        merge_proxy_paths(&mut tainted, *base, paths);
                    }
                    _ => {}
                }
                let paths = match &op.kind {
                    OpKind::Copy(value)
                    | OpKind::Move(value)
                    | OpKind::Deref(value)
                    | OpKind::Materialize(value)
                    | OpKind::Convert { v: value, .. } => proxy_paths(&tainted, *value),
                    OpKind::Aggregate { fields, .. } => fields
                        .iter()
                        .enumerate()
                        .flat_map(|(field, value)| {
                            prefixed_proxy_paths(
                                proxy_paths(&tainted, *value),
                                u32::try_from(field).unwrap_or(u32::MAX),
                            )
                        })
                        .collect(),
                    OpKind::Field { base, field, .. } => {
                        projected_proxy_paths(proxy_paths(&tainted, *base), *field)
                    }
                    OpKind::CellLoad(cell) => tainted_cells.get(cell).cloned().unwrap_or_default(),
                    _ => ProxyPaths::new(),
                };
                if let Some(result) = op.result {
                    merge_proxy_paths(&mut tainted, result, paths);
                }
            }
            for (successor, arguments) in block.term.edges() {
                let Some(target) = func.blocks.get(successor.0 as usize) else {
                    return false;
                };
                for (target, argument) in target.params.iter().zip(arguments) {
                    let paths = proxy_paths(&tainted, *argument);
                    merge_proxy_paths(&mut tainted, *target, paths);
                }
            }
        }
        if tainted == before_values && tainted_cells == before_cells {
            break;
        }
    }

    for block in &func.blocks {
        for op in &block.ops {
            let uses_proxy = op
                .kind
                .uses()
                .iter()
                .any(|value| !proxy_paths(&tainted, *value).is_empty());
            if !uses_proxy {
                continue;
            }
            match &op.kind {
                OpKind::Copy(_)
                | OpKind::Move(_)
                | OpKind::Aggregate { .. }
                | OpKind::Field { .. }
                | OpKind::CellLoad(_)
                | OpKind::CellStore { .. }
                | OpKind::Deref(_)
                | OpKind::Materialize(_)
                | OpKind::Convert { .. } => {}
                OpKind::SetField { base, .. }
                    if !proxy_paths(&tainted, *base).contains(&Vec::new()) => {}
                OpKind::Unary { v, .. } if proxy_paths(&tainted, *v).iter().all(Vec::is_empty) => {}
                OpKind::Binary { op: binary, .. }
                    if !matches!(binary, BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Cmp)
                        && op
                            .kind
                            .uses()
                            .iter()
                            .flat_map(|value| proxy_paths(&tainted, *value))
                            .all(|path| path.is_empty()) => {}
                OpKind::Call {
                    function,
                    args,
                    host: Some(host),
                    ..
                } if function.is_none() && proxy_callable_host(host, item_interns) => {
                    for argument in args {
                        let paths = proxy_paths(&tainted, *argument);
                        if paths.is_empty() {
                            continue;
                        }
                        if !proxy_closure_safe(func, *argument, paths, item_interns, program, cache)
                        {
                            return false;
                        }
                    }
                }
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    function: None,
                    args,
                    host: None,
                    ..
                } => {
                    let Some(entry) = program.items.get(item) else {
                        return false;
                    };
                    let target = JsName::new(entry.name.clone());
                    for (parameter, argument) in args.iter().enumerate() {
                        let paths = proxy_paths(&tainted, *argument);
                        if paths.is_empty() {
                            continue;
                        }
                        let Ok(parameter) = u16::try_from(parameter) else {
                            return false;
                        };
                        if !proxy_parameter_safe(&target, parameter, paths, program, cache) {
                            return false;
                        }
                    }
                }
                _ => return false,
            }
        }
        match &block.term {
            RTerm::Invoke { .. } => return false,
            RTerm::Goto(..) | RTerm::Unreachable => {}
            RTerm::Switch { scrut, .. } if !proxy_paths(&tainted, *scrut).is_empty() => {
                return false;
            }
            RTerm::Return(Some(value)) | RTerm::Throw(Some(value))
                if !proxy_paths(&tainted, *value).is_empty() =>
            {
                return false;
            }
            RTerm::Switch { .. } | RTerm::Return(_) | RTerm::Throw(_) => {}
        }
    }
    true
}

fn root_proxy_paths() -> ProxyPaths {
    BTreeSet::from([Vec::new()])
}

fn proxy_paths<K: Ord>(values: &BTreeMap<K, ProxyPaths>, value: K) -> ProxyPaths {
    values.get(&value).cloned().unwrap_or_default()
}

fn merge_proxy_paths<K: Ord>(values: &mut BTreeMap<K, ProxyPaths>, value: K, paths: ProxyPaths) {
    if paths.is_empty() {
        return;
    }
    values.entry(value).or_default().extend(paths);
}

fn prefixed_proxy_paths(paths: ProxyPaths, field: u32) -> ProxyPaths {
    paths
        .into_iter()
        .map(|mut path| {
            path.insert(0, field);
            path
        })
        .collect()
}

fn projected_proxy_paths(paths: ProxyPaths, field: u32) -> ProxyPaths {
    paths
        .into_iter()
        .filter_map(|mut path| {
            (path.first() == Some(&field)).then(|| {
                path.remove(0);
                path
            })
        })
        .collect()
}

fn proxy_callable_host(host: &HostPayload, interns: &crate::jir::Interns) -> bool {
    if matches!(host, HostPayload::KeyedList(_)) {
        return true;
    }
    let marker = match host {
        HostPayload::ViewAbi { marker, .. } => interns.str(*marker),
        HostPayload::Marker { section, .. } => interns
            .str(*section)
            .and_then(|section| section.strip_prefix("rcgjs.tc.")),
        _ => None,
    };
    matches!(marker, Some("handler" | "effect" | "component" | "cond"))
}

fn proxy_closure_safe(
    func: &Func,
    closure: ValueId,
    paths: ProxyPaths,
    item_interns: &crate::jir::Interns,
    program: &ProxyProgram<'_, '_>,
    cache: &mut ProxyCache,
) -> bool {
    let Some(value) = func.values.get(closure.0 as usize) else {
        return false;
    };
    let peeled = item_interns.types.peeled(value.ty);
    let Some(TypeKind::Closure { def, .. }) = item_interns
        .types
        .get(peeled)
        .map(|descriptor| &descriptor.kind)
    else {
        return false;
    };
    let item = ItemId {
        definition: *def,
        instantiation: peeled.0,
    };
    let Some(entry) = program.items.get(&item) else {
        return false;
    };
    proxy_parameter_safe(&JsName::new(entry.name.clone()), 0, paths, program, cache)
}

/// Every vtable item of the program by the `(concrete type, principal trait)` pair it is the
/// vtable of, which is what an unsizing coercion to `dyn` names (`lower/js/unsize.rs`).
///
/// Built the same way and for the same reason as [`item_index`]: v1 asked its namer for the name,
/// and the only thing that knows it here is the vtable item itself.
fn vtable_index(table: &BTreeMap<JsName, LinkItem>) -> VtableIndex {
    let mut index = VtableIndex::new();
    for (name, item) in table {
        let Some(body) = item.jir.body.as_ref() else {
            continue;
        };
        let crate::jir::BodyKind::Vtable {
            concrete,
            trait_ref,
            ..
        } = &body.kind
        else {
            continue;
        };
        index.insert((*concrete, *trait_ref), name.as_str().to_owned());
    }
    index
}

/// Installs one lowering into its link item, and returns the bytes it printed and the bytes it
/// would take minified.
///
/// Printing is v1's exact path: the declaration goes into a [`JsItem`], through
/// `minify::shorten_locals` and `JsItem::to_js_mapped`, which is the pair the backend's own
/// `driver.rs` runs the baseline through. A lane that printed by another route would make every
/// golden a comparison of two printers rather than of two lowerings.
fn apply(
    dcx: &Sink,
    item: &mut LinkItem,
    mut lowered: LoweredItem,
    defined: &BTreeSet<JsName>,
    live: &BTreeSet<JsName>,
) -> Result<Applied, Decline> {
    // `js-str=bytes`: the lowering spoke the hybrid's helpers; the installed item speaks the
    // byte-string ones (`lower/bstr.rs`).
    // A helper that became a property read (`str_len`) or nothing (`native_str_wf`) takes its
    // `__rt` mention with it, so the refs are the rewritten declaration's.
    if crate::lower::bstr::enabled() {
        crate::lower::bstr::rewrite(&mut lowered.decl);
        crate::lower::js::function::fold_bstr_in_bytes_decl(&mut lowered.decl);
        lowered.refs = refs_of(&lowered.decl, &item.name);
    }
    // Before `shorten_locals`, which renames locals and so changes the identifier set: the
    // baseline's `refs` was taken at the same point in the backend.
    let refs = refs_of(&lowered.decl, &item.name);
    let refs_changed = match refusal(item, &lowered, &refs, defined, live) {
        Ok(changed) => changed,
        Err((message, what)) => {
            dcx.err(message);
            return Err(refs_disagreement(what));
        }
    };
    let mut printed = JsItem {
        name: item.name.clone(),
        kind: item.kind,
        decl: lowered.decl,
        refs,
        zombies: Vec::new(),
        linkage: item.linkage.clone(),
        debug_path: item.debug_path.clone(),
        order: item.order.clone(),
        loc: lowered.loc,
        names: lowered.names,
        dts: lowered.dts,
        optimization: Default::default(),
        late_ir: Default::default(),
        baseline_shadow: Default::default(),
        jir: None,
        jir_decline: None,
        jir_item: None,
    };
    crate::minify::shorten_locals(std::slice::from_mut(&mut printed));
    // What the item costs minified, whether or not this build minifies: a size claim about one
    // item must not need `js-minify` on the whole program, which changes every other item too.
    // Measured on a clone, because a program whose locals were shortened here and nowhere else
    // would be half-minified.
    let mut measured = printed.decl.clone();
    crate::minify::shorten_locals_for_measurement(&mut measured);
    let bytes_min = jsast::program_to_string_styled_for_target(
        std::slice::from_ref(&measured),
        jsast::Style::Compact,
        crate::opts::get().ecma_target,
    )
    .len();
    let (js, mappings) = printed.to_js_mapped(crate::opts::get().comments);

    // The item now references what ITS body references, which is what `stub_missing`, the chunk
    // walk and any later `reach` read. Taken before `shorten_locals` for the reason the backend
    // takes it there: a renamed local is not a new reference.

    item.js = js;
    item.refs = std::mem::take(&mut printed.refs);
    item.map = item_map(mappings);
    // A `.d.ts` signature is produced by a type walk the lowerer does not run yet, so `None`
    // means "nothing to say about it" rather than "this item has no signature": the baseline's
    // is kept. `kind`, `linkage` and `order` are never a lane's to change at all — they are what
    // the item IS, and printing it differently does not move it in the program.
    if printed.dts.is_some() {
        item.dts = printed.dts;
    }
    item.zombies = lowered.zombies;
    Ok(Applied {
        bytes: item.js.len(),
        bytes_min,
        refs_changed,
    })
}

/// What installing one lowering cost, and whether it moved the program's reachability.
struct Applied {
    /// The bytes the lowering printed.
    bytes: usize,
    /// The bytes it takes printed compact.
    bytes_min: usize,
    /// Whether its linkable references differ from the baseline's.
    refs_changed: bool,
}

/// Whether the driver may install `lowered` over `item`, and whether doing so moves what the item
/// reaches. `Err` is the message to report and the short name of the rule that refused it.
///
/// Split out of [`apply`] so phase C can ask the question before the table is touched: a lowering
/// the driver is going to refuse leaves the item on its baseline text, and for an item the ABI
/// plan rewrote that is the shape mismatch the retry exists to prevent.
///
/// What a lowering may reference is not "whatever the baseline referenced" but "whatever the link
/// step already prepared". Two directions, and they are asymmetric on purpose:
///
/// * FEWER than the baseline is always fine. A lowering that dropped a reference proved something
///   dead, and a walk given fewer edges visits fewer items — all of them ones the walk already
///   visited.
/// * MORE than the baseline is fine too, as long as the item is already **reachable**: the walk
///   decoded it, `order` will place it and `print_units` will print it, so referencing it costs
///   nothing. The JIR lowering of an item legitimately reaches names the baseline spelled another
///   way — `__rt`, the ESM shim import, is the common one.
///
/// What is refused is a reference to something the link step did NOT prepare: an item defined but
/// never reached (it will not be printed, so the reference dangles) or a namer-shaped `$h` name no
/// item defines and the baseline never mentioned (it would grow the program by a `stub_missing`
/// throwing stub that nothing asked for).
fn refusal(
    item: &LinkItem,
    lowered: &LoweredItem,
    refs: &BTreeSet<JsName>,
    defined: &BTreeSet<JsName>,
    live: &BTreeSet<JsName>,
) -> Result<bool, (String, &'static str)> {
    if refs != &lowered.refs {
        return Err((
            format!(
                "rustc_codegen_js: the JIR lowering of `{}` declared references its declaration \
                 does not make; this is a compiler bug, please report it",
                item.name.as_str()
            ),
            "the lowering's declared refs",
        ));
    }
    let lowered_links = linkable(refs, defined);
    let baseline_links = linkable(&item.refs, defined);
    let unprepared: Vec<&&JsName> = lowered_links
        .iter()
        .filter(|name| !live.contains(**name) && !baseline_links.contains(*name))
        .collect();
    if !unprepared.is_empty() {
        return Err((
            format!(
                "rustc_codegen_js: the JIR lowering of `{}` references {unprepared:?}, which the \
                 link step did not prepare: not reachable, and not referenced by the baseline \
                 either. This is a compiler bug, please report it",
                item.name.as_str(),
            ),
            "what the link step prepared",
        ));
    }
    Ok(lowered_links != baseline_links)
}

/// The decline recorded for an item whose two lowerings do not agree about what they reference.
fn refs_disagreement(what: &str) -> Decline {
    Decline::Lower(LowerDecline::Other {
        op: OpClass::Other,
        detail: format!("the JIR lowering disagrees with {what}"),
    })
}

/// The subset of `refs` that has any effect on the program.
///
/// `refs` contains free identifier uses. Reachability follows names the item table defines;
/// `stub_missing` supplies a throwing body for an undefined namer-shaped `$h` name. Other free
/// names, such as JavaScript builtins, do not create dependencies between linked items.
fn linkable<'a>(refs: &'a BTreeSet<JsName>, defined: &BTreeSet<JsName>) -> BTreeSet<&'a JsName> {
    refs.iter()
        .filter(|name| defined.contains(*name) || super::is_namer_name(name.as_str()))
        .collect()
}

/// Every free identifier `decl` mentions, its own name excluded: `JsItem::new`'s rule, which is
/// what the baseline's `refs` was built with.
pub(super) fn refs_of(decl: &jsast::Stmt, name: &JsName) -> BTreeSet<JsName> {
    let mut refs = BTreeSet::new();
    jsast::visit_free_idents(decl, &mut |ident| {
        if ident != name.as_str() {
            refs.insert(JsName::new(ident.to_owned()));
        }
    });
    refs
}

/// The compiled-in allow list, comments and blank lines removed.
fn allowlist() -> BTreeSet<&'static str> {
    ALLOWLIST
        .lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ItemKind, ItemMap, Linkage};

    #[test]
    fn a_transitively_hot_callee_charges_every_site_at_loop_depth_one() {
        assert_eq!(propagated_alloc_term(3, 72, true, 24), 288);
        assert_eq!(propagated_alloc_term(3, 1_200, true, 24), 1_200);
        assert_eq!(propagated_alloc_term(3, 72, false, 24), 72);
    }

    #[test]
    fn keyed_row_capture_keeps_only_creation_time_view_markers() {
        let mut interns = crate::jir::Interns::default();
        let definition = crate::jir::semantic::DefinitionId {
            crate_id: 0,
            local_hash: 1,
        };
        let marker = |name: &str, interns: &mut crate::jir::Interns| HostPayload::ViewAbi {
            marker: interns.intern_str(name),
            def: definition,
            helpers: Vec::new(),
        };
        assert_eq!(
            keyed_host_capture(&marker("hole", &mut interns), &interns),
            RowCaptureProof::Static
        );
        assert_eq!(
            keyed_host_capture(&marker("effect", &mut interns), &interns),
            RowCaptureProof::Live
        );
        assert_eq!(
            keyed_host_capture(&marker("handler", &mut interns), &interns),
            RowCaptureProof::Live
        );
        assert_eq!(
            RowCaptureProof::Static.combine(RowCaptureProof::Unknown),
            RowCaptureProof::Unknown
        );
        assert_eq!(
            RowCaptureProof::Unknown.combine(RowCaptureProof::Live),
            RowCaptureProof::Live
        );
    }

    fn item(name: &str, refs: &[&str]) -> LinkItem {
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind: ItemKind::Fn,
            js: format!("\nfunction {name}() {{\n  return 1;\n}}\n"),
            refs: refs
                .iter()
                .map(|name| JsName::new((*name).to_owned()))
                .collect(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: format!("test::{name}"),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: Default::default(),
        }
    }

    /// The declaration `function <name>() { return <callee>(); }`, and the refs it makes.
    fn lowering(name: &str, callee: &str) -> LoweredItem {
        let decl = jsast::function(
            name.to_owned(),
            Vec::new(),
            vec![jsast::Stmt::Return(Some(jsast::call(
                jsast::Expr::Ident(callee.to_owned()),
                Vec::new(),
            )))],
        );
        LoweredItem {
            refs: [JsName::new(callee.to_owned())].into_iter().collect(),
            decl,
            names: Vec::new(),
            loc: None,
            dts: None,
            zombies: Vec::new(),
            js_alloc_origins: Vec::new(),
            bigints: BTreeSet::new(),
        }
    }

    /// An item with no decoded body is what a synthesized item looks like — the allocator shim,
    /// a link-step stub — and it must never be offered, never counted, never a fallback.
    #[test]
    fn an_item_without_a_body_is_never_offered() {
        let dcx = Sink::new();
        let mut table = BTreeMap::new();
        let one = item("one$h1", &[]);
        table.insert(one.name.clone(), one);
        let reachable = vec![JsName::new("one$h1".to_owned())];
        let run = run(&dcx, &mut table, &reachable, &Default::default());
        assert_eq!(run.fallback_count(), 0);
        assert!(run.outcome(&JsName::new("one$h1".to_owned())).is_none());
        assert_eq!(dcx.errors(), 0);
    }

    #[test]
    fn a_lowering_replaces_the_printed_text_and_the_map() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &["two$h2"]);
        let baseline = one.js.clone();
        let bytes = apply(
            &dcx,
            &mut one,
            lowering("one$h1", "two$h2"),
            &defined(&["one$h1", "two$h2"]),
            &defined(&["one$h1", "two$h2"]),
        )
        .expect("applied");
        assert_eq!(dcx.errors(), 0);
        assert_ne!(one.js, baseline);
        assert_eq!(one.js.len(), bytes.bytes);
        assert!(one.js.contains("return two$h2();"), "{}", one.js);
        // The header comment is the printer's, from the item's own debug path: the lane prints
        // through `JsItem::to_js_mapped`, exactly as the baseline was printed.
        assert!(one.js.starts_with("// test::one$h1\n"), "{}", one.js);
    }

    /// The item table the driver holds the two lanes to: the names a program defines.
    fn defined(names: &[&str]) -> BTreeSet<JsName> {
        names
            .iter()
            .map(|name| JsName::new((*name).to_owned()))
            .collect()
    }

    #[test]
    fn a_lowering_that_reaches_further_than_the_baseline_is_a_hard_error() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &["two$h2"]);
        let baseline = one.js.clone();
        let result = apply(
            &dcx,
            &mut one,
            lowering("one$h1", "three$h3"),
            &defined(&["one$h1", "two$h2", "three$h3"]),
            // `three$h3` is defined but was never reached, so nothing will print it.
            &defined(&["one$h1", "two$h2"]),
        );
        assert!(result.is_err());
        assert_eq!(dcx.errors(), 1);
        assert_eq!(one.js, baseline, "the item must keep its baseline text");
    }

    /// A lowering whose declared `refs` are not the ones its declaration makes is caught before
    /// the baseline comparison: the two would otherwise cancel out.
    #[test]
    fn a_lowering_that_misdeclares_its_own_refs_is_a_hard_error() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &["two$h2"]);
        let mut lowered = lowering("one$h1", "two$h2");
        lowered.refs = BTreeSet::new();
        assert!(apply(
            &dcx,
            &mut one,
            lowered,
            &defined(&["one$h1", "two$h2"]),
            &defined(&["one$h1", "two$h2"])
        )
        .is_err());
        assert_eq!(dcx.errors(), 1);
    }

    /// A lowering that reaches FEWER items than the baseline is allowed: it proved something
    /// dead, and a walk that already visited a superset has nothing to redo.
    #[test]
    fn a_lowering_may_reach_fewer_items_than_the_baseline() {
        let dcx = Sink::new();
        // The baseline calls `two$h2` and `three$h3`; the lowering folded the arm that called
        // `three$h3` away.
        let mut one = item("one$h1", &["two$h2", "three$h3"]);
        let bytes = apply(
            &dcx,
            &mut one,
            lowering("one$h1", "two$h2"),
            &defined(&["one$h1", "two$h2", "three$h3"]),
            &defined(&["one$h1", "two$h2", "three$h3"]),
        )
        .expect("applied");
        assert_eq!(dcx.errors(), 0);
        assert_eq!(one.js.len(), bytes.bytes);
        assert!(!one.js.contains("three$h3"), "{}", one.js);
        // The item's own refs are the ones its printed body makes, so the next walk sees them.
        assert!(!one.refs.contains(&JsName::new("three$h3".to_owned())));
        // And the walk has to run again, or `three$h3` stays in a program nothing calls it from.
        assert!(bytes.refs_changed);
    }

    /// The link step re-walks reachability when a lowering the lane KEPT reaches something else
    /// than its baseline did. A row that was refused says nothing about the program: the item
    /// kept its baseline text, which reaches what it always did.
    #[test]
    fn a_run_reports_a_reference_change_only_for_an_item_it_kept() {
        let row = |lane, refs_equal| LaneOutcome {
            name: JsName::new("one$h1".to_owned()),
            class: ItemClass::empty(),
            lane,
            decline: None,
            refs_equal,
            bytes_baseline: 1,
            bytes_jir: 1,
            bytes_min: 1,
            alloc_json: None,
            js_alloc_json: None,
            host_args: Default::default(),
            slice_iterator_would_lower: 0,
            slice_iterator_final_emitted: 0,
        };
        let run = |outcome| LaneRun {
            outcomes: vec![outcome],
            ..LaneRun::default()
        };
        assert!(run(row(ItemLane::Jir, false)).references_changed());
        assert!(!run(row(ItemLane::Jir, true)).references_changed());
        assert!(!run(row(ItemLane::Fallback, false)).references_changed());
    }

    /// The JIR lowering of an item legitimately reaches names the baseline spelled another way —
    /// `__rt`, the ESM shim import, is the one this rule was written for. Referencing an item the
    /// walk already reached costs nothing: it is decoded, ordered and printed either way.
    #[test]
    fn a_lowering_may_reach_an_item_the_walk_already_prepared() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &[]);
        let bytes = apply(
            &dcx,
            &mut one,
            lowering("one$h1", "__rt"),
            &defined(&["one$h1", "__rt"]),
            &defined(&["one$h1", "__rt"]),
        )
        .expect("applied");
        assert_eq!(dcx.errors(), 0);
        assert_eq!(one.js.len(), bytes.bytes);
        assert!(one.refs.contains(&JsName::new("__rt".to_owned())));
    }

    /// A namer-shaped name no item defines is one `stub_missing` synthesizes a throwing body for,
    /// so introducing one grows the program by an item. Refused like any other new reference.
    #[test]
    fn a_lowering_that_invents_a_namer_shaped_name_is_a_hard_error() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &["two$h2"]);
        let result = apply(
            &dcx,
            &mut one,
            lowering("one$h1", "never$h0000000000000009"),
            &defined(&["one$h1", "two$h2"]),
            &defined(&["one$h1", "two$h2"]),
        );
        assert!(result.is_err());
        assert_eq!(dcx.errors(), 1);
    }

    /// The two lanes name their temporaries differently, and that is the point of having two.
    /// Only what the item table defines is held equal.
    #[test]
    fn a_lowering_may_name_its_locals_its_own_way() {
        let dcx = Sink::new();
        // The baseline named its temporary `_13`; the lowering names the same one `$v103`.
        let mut one = item("one$h1", &["two$h2", "_13"]);
        let mut lowered = lowering("one$h1", "two$h2");
        // `function one$h1() { let $v103 = 1; return two$h2($v103); }`
        let jsast::Stmt::FunctionDecl { body, .. } = &mut lowered.decl else {
            panic!("the lowering is a function declaration");
        };
        body.insert(0, jsast::Stmt::Let("$v103".to_owned(), Some(jsast::num(1))));
        *body.last_mut().expect("a return") = jsast::Stmt::Return(Some(jsast::call(
            jsast::Expr::Ident("two$h2".to_owned()),
            vec![jsast::Expr::Ident("$v103".to_owned())],
        )));
        let bytes = apply(
            &dcx,
            &mut one,
            lowered,
            &defined(&["one$h1", "two$h2"]),
            &defined(&["one$h1", "two$h2"]),
        )
        .expect("applied");
        assert_eq!(dcx.errors(), 0);
        assert!(one.js.contains("$v103"), "{}", one.js);
        assert_eq!(one.js.len(), bytes.bytes);
    }

    /// `$high` is a field lane suffix, not the readable namer's 16-hex hash suffix.
    #[test]
    fn a_physicalized_local_is_not_a_namer_shaped_reference() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &["two$h2"]);
        let mut lowered = lowering("one$h1", "two$h2");
        let jsast::Stmt::FunctionDecl { body, .. } = &mut lowered.decl else {
            panic!("the lowering is a function declaration");
        };
        body.insert(
            0,
            jsast::Stmt::Let("acc$high".to_owned(), Some(jsast::num(1))),
        );
        *body.last_mut().expect("a return") = jsast::Stmt::Return(Some(jsast::call(
            jsast::Expr::Ident("two$h2".to_owned()),
            vec![jsast::Expr::Ident("acc$high".to_owned())],
        )));

        apply(
            &dcx,
            &mut one,
            lowered,
            &defined(&["one$h1", "two$h2"]),
            &defined(&["one$h1", "two$h2"]),
        )
        .expect("applied");
        assert_eq!(dcx.errors(), 0);
    }

    #[test]
    fn a_local_matching_an_unprepared_item_is_not_a_dependency() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &["two$h2"]);
        let mut lowered = lowering("one$h1", "two$h2");
        let jsast::Stmt::FunctionDecl { body, .. } = &mut lowered.decl else {
            panic!("a function declaration");
        };
        body.insert(0, jsast::let_("log", jsast::num(1)));
        *body.last_mut().expect("return") =
            jsast::ret(jsast::call(jsast::id("two$h2"), vec![jsast::id("log")]));
        apply(
            &dcx,
            &mut one,
            lowered,
            &defined(&["one$h1", "two$h2", "log"]),
            &defined(&["one$h1", "two$h2"]),
        )
        .expect("local is not an unprepared dependency");
        assert!(!one.refs.contains(&JsName::new("log".into())));
        assert_eq!(dcx.errors(), 0);
    }

    /// The lowerer produces no `.d.ts` yet, and `None` must not erase the one the object carried.
    /// Nor may a lane touch what the item IS: its kind, its linkage, its declaration order.
    #[test]
    fn a_lowering_without_a_signature_keeps_the_baseline_one() {
        let dcx = Sink::new();
        let mut one = item("one$h1", &["two$h2"]);
        one.dts = Some(crate::dts::Dts {
            params: vec![("a".to_owned(), "number".to_owned())],
            ret: "number".to_owned(),
            types: Vec::new(),
            rust: "fn one(a: u32) -> u32".to_owned(),
        });
        apply(
            &dcx,
            &mut one,
            lowering("one$h1", "two$h2"),
            &defined(&["one$h1", "two$h2"]),
            &defined(&["one$h1", "two$h2"]),
        )
        .expect("applied");
        assert_eq!(
            one.dts.as_ref().map(|dts| dts.rust.as_str()),
            Some("fn one(a: u32) -> u32")
        );
        assert_eq!(one.kind, ItemKind::Fn);
        assert!(one.order.is_none());
    }

    #[test]
    fn the_sidecar_is_the_shape_the_shadow_script_reads() {
        let run = LaneRun {
            passes: Passes {
                would_apply: PassCounts {
                    sroa: 1,
                    ast_dce: 2,
                    inline: 1,
                    devirt: 0,
                },
                applied: PassCounts {
                    sroa: 1,
                    ast_dce: 2,
                    inline: 1,
                    devirt: 0,
                },
            },
            abi_json: Some("{\"applied\":true,\"registers\":0}".to_owned()),
            repr: None,
            plan_retries: 0,
            added_items: false,
            outcomes: vec![
                LaneOutcome {
                    name: JsName::new("a$h1".to_owned()),
                    class: ItemClass::SCALAR_LEAF,
                    lane: ItemLane::Jir,
                    decline: None,
                    refs_equal: true,
                    bytes_baseline: 40,
                    bytes_jir: 36,
                    bytes_min: 28,
                    alloc_json: Some("{\"materialize\":1}".to_owned()),
                    js_alloc_json: Some("{\"object\":1}".to_owned()),
                    host_args: Default::default(),
                    slice_iterator_would_lower: 2,
                    slice_iterator_final_emitted: 2,
                },
                LaneOutcome {
                    name: JsName::new("b$h2".to_owned()),
                    class: ItemClass::REFS.union(ItemClass::UNWIND),
                    lane: ItemLane::Fallback,
                    decline: Some(Decline::Lower(LowerDecline::Other {
                        op: OpClass::Other,
                        detail: "jir lowering not implemented".to_owned(),
                    })),
                    refs_equal: true,
                    bytes_baseline: 90,
                    bytes_jir: 0,
                    bytes_min: 0,
                    alloc_json: None,
                    js_alloc_json: None,
                    host_args: Default::default(),
                    slice_iterator_would_lower: 1,
                    slice_iterator_final_emitted: 0,
                },
            ],
            ..LaneRun::default()
        };
        assert_eq!(run.fallback_count(), 1);
        assert_eq!(
            run.render(Lane::Shadow),
            "{\"schema\":17,\"lane\":\"shadow\",\"fallback_count\":1,\
             \"passes\":{\"would_apply\":{\"sroa\":1,\"ast_dce\":2,\"inline\":1,\"devirt\":0},\
             \"applied\":{\"sroa\":1,\"ast_dce\":2,\"inline\":1,\"devirt\":0}},\
             \"summaries\":{\"items\":0,\"validated\":0,\"top\":0},\
             \"inline\":{\"applied\":0,\"cross_object_applied\":0,\"declined\":{},\"declined_budget\":0,\"bytes_before\":0,\"bytes_after\":0},\
             \"devirt\":{\"applied\":0},\
             \"range\":{\"asserts_removed\":0,\"comparisons_folded\":0},\
             \"slice_iter\":{\"would_lower\":3,\"final_emitted\":2},\
             \"witnesses\":{\"recorded\":0,\"failed\":0},\
             \"plan_retries\":0,\
             \"abi\":{\"applied\":true,\"registers\":0},\"physical\":null,\"async\":null,\
             \"repr\":null,\"items\":[\
             {\"name\":\"a$h1\",\"class\":[\"scalar-leaf\"],\"lane\":\"jir\",\"decline\":null,\
             \"refs_equal\":true,\"bytes_baseline\":40,\"bytes_jir\":36,\"bytes_min\":28,\
             \"slice_iter\":{\"would_lower\":2,\"final_emitted\":2},\
             \"host_args\":{\"declines\":{},\"final_emitted\":0,\"would_lower\":0},\
             \"alloc\":{\"materialize\":1},\"js_alloc\":{\"object\":1}},\
             {\"name\":\"b$h2\",\"class\":[\"refs\",\"unwind\"],\"lane\":\"fallback\",\
             \"decline\":\"lower.other\",\"refs_equal\":true,\"bytes_baseline\":90,\
             \"bytes_jir\":0,\"bytes_min\":0,\
             \"slice_iter\":{\"would_lower\":1,\"final_emitted\":0},\
             \"host_args\":{\"declines\":{},\"final_emitted\":0,\"would_lower\":0},\
             \"alloc\":null,\"js_alloc\":null}]}\n"
        );
    }

    /// The `repr` block renders D12's exact shape, decisions included, so the exit gate and
    /// `scripts/repr-report-test.sh` read one stable schema.
    #[test]
    fn the_repr_block_is_the_shape_the_gate_reads() {
        let run = LaneRun {
            repr: Some(ReprBlock {
                families: 3,
                candidates: 4,
                legal: 3,
                cost_positive: 2,
                committed: 2,
                planned: 2,
                applied: 2,
                decisions: vec![
                    ReprDecision {
                        subject: "a$h1/v3".to_owned(),
                        family: "string".to_owned(),
                        chosen: "host-string".to_owned(),
                        alternatives: vec!["host-string".to_owned(), "string-builder".to_owned()],
                        decline: None,
                        items: [JsName::new("a$h1".to_owned())].into_iter().collect(),
                    },
                    ReprDecision {
                        subject: "b$h2/v1".to_owned(),
                        family: "carrier".to_owned(),
                        chosen: "baseline".to_owned(),
                        alternatives: vec!["baseline".to_owned(), "native-vec".to_owned()],
                        decline: Some("cost.no-improvement before=120 after=140".to_owned()),
                        items: [JsName::new("b$h2".to_owned())].into_iter().collect(),
                    },
                ],
                adapters: 1,
                versions: 1,
                helper_bytes: 512,
                pinned_cost_positive: 1,
                exports: vec![ReprExport {
                    subject: "give$h3/ret".to_owned(),
                    chosen: "host-string".to_owned(),
                }],
                bytes_before: 1_000,
                bytes_after: 900,
                score_before: 1_100,
                score_after: 950,
                emits: true,
            }),
            ..LaneRun::default()
        };
        let rendered = run.render(Lane::Jir);
        assert!(
            rendered.contains(
                "\"repr\":{\"families\":3,\"planned\":{\"candidates\":4,\"legal\":3,\
                 \"would_commit\":2,\"pinned_cost_positive\":1},\
                 \"validated\":{\"cost_positive\":2,\"would_commit\":2,\"adapters\":1,\
                 \"versions\":1,\"helper_bytes\":512,\"bytes_before\":1000,\
                 \"bytes_after\":900,\"score_before\":1100,\"score_after\":950},\
                 \"final_emitted\":{\"applied\":2,\"bytes\":900},\"decisions\":[\
                 {\"subject\":\"a$h1/v3\",\"family\":\"string\",\"chosen\":\"host-string\",\
                 \"alternatives\":[\"host-string\",\"string-builder\"],\"decline\":null},\
                 {\"subject\":\"b$h2/v1\",\"family\":\"carrier\",\"chosen\":\"baseline\",\
                 \"alternatives\":[\"baseline\",\"native-vec\"],\
                 \"decline\":\"cost.no-improvement before=120 after=140\"}],\
                 \"exports\":[{\"subject\":\"give$h3/ret\",\"chosen\":\"host-string\"}]}"
            ),
            "{rendered}"
        );
    }

    /// `link_dedup` may retire an item the lane lowered; the sidecar describes what was printed.
    #[test]
    fn a_row_whose_item_stopped_being_printed_is_dropped() {
        let mut run = LaneRun {
            outcomes: vec![LaneOutcome {
                name: JsName::new("gone$h1".to_owned()),
                class: ItemClass::empty(),
                lane: ItemLane::Jir,
                decline: None,
                refs_equal: true,
                bytes_baseline: 1,
                bytes_jir: 1,
                bytes_min: 1,
                alloc_json: None,
                js_alloc_json: None,
                host_args: Default::default(),
                slice_iterator_would_lower: 1,
                slice_iterator_final_emitted: 1,
            }],
            ..LaneRun::default()
        };
        run.retain_reachable(&[JsName::new("other$h2".to_owned())]);
        assert!(run.render(Lane::Jir).contains("\"items\":[]"));
    }

    #[test]
    fn the_allow_list_is_comments_only_today() {
        assert!(
            allowlist().is_empty(),
            "adding a name to examples/jir-allowlist.txt stops the exit gate watching it; say \
             who is watching it instead, and update this test with the reason"
        );
    }

    #[test]
    fn the_sidecar_and_shadow_paths_sit_beside_the_program() {
        let path = Path::new("/build/program.js");
        assert_eq!(sidecar_path(path), Path::new("/build/program.js.lane.json"));
        assert_eq!(shadow_path(path), Path::new("/build/program.js.jir.js"));
    }

    #[test]
    fn abi_fault_retry_matches_flat_abi_off_with_post_repr_vec_push() {
        use crate::jir::origin::{Origin, OriginKind, OriginTable};
        use crate::jir::semantic::{
            AbiSignature, AggregateKind, BasicBlock, BinaryOp, BlockId, Body, BodyKind,
            BodySemantic, Call, CallSemantic, Callee, Constant, DefinitionId, DirectItemRef, Local,
            LocalId, LocalRole, Operand, OperandKind, Place, Projection, Rvalue, Signature,
            StandardOperation, Statement, StatementKind, Terminator, Unwind,
        };
        use crate::jir::types::{
            FieldDescriptor, FieldKey, IntFacts, IntRepr, JsFacts, LayoutFacts, TypeDescriptor,
            TypeId, TypeKind, TypeMeta,
        };
        use crate::jir::Interns;
        use crate::jir::OriginId;
        use crate::opts::{Lane, OptimizerMode, Opts};
        use rcgjs_object::item::{read_object, write_object, JirItem, JsItem, ObjectIdentity};

        const U32: TypeId = TypeId(1);
        const PAIR: TypeId = TypeId(2);
        const FN_FLAT: TypeId = TypeId(3);
        const FN_VEC_PUSH: TypeId = TypeId(4);

        let fn_item = |local_hash: u64| ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash,
            },
            instantiation: u128::from(local_hash),
        };
        let flat_id = fn_item(2);
        let push_id = fn_item(3);

        let mut interns = Interns::new();
        let flat_name = interns.intern_str("flat_callee$h2");
        let push_name = interns.intern_str("vec_push$h3");
        let empty_js_facts = |peeled: TypeId| JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        };
        let u32_desc = TypeDescriptor {
            id: U32,
            name: None,
            kind: TypeKind::Integer {
                signed: false,
                bits: 32,
            },
            layout: LayoutFacts {
                size: Some(4),
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int: Some(IntFacts {
                    signed: false,
                    bits: 32,
                    repr: IntRepr::Number,
                }),
                ..empty_js_facts(U32)
            },
        };
        let pair_desc = TypeDescriptor {
            id: PAIR,
            name: None,
            kind: TypeKind::Tuple(vec![U32, U32]),
            layout: LayoutFacts {
                size: Some(8),
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                is_indirect: true,
                needs_clone: true,
                fields: vec![
                    FieldDescriptor {
                        key: FieldKey::Index(0),
                        ty: U32,
                        offset: 0,
                    },
                    FieldDescriptor {
                        key: FieldKey::Index(1),
                        ty: U32,
                        offset: 4,
                    },
                ],
                ..empty_js_facts(PAIR)
            },
        };
        let fn_desc = |id: TypeId, local_hash: u64, item: ItemId, link_name| TypeDescriptor {
            id,
            name: None,
            kind: TypeKind::FnDef {
                def: DefinitionId {
                    crate_id: 0,
                    local_hash,
                },
                sig: crate::jir::interns::SigId(0),
                instance: Some(DirectItemRef { item, link_name }),
            },
            layout: LayoutFacts {
                size: Some(0),
                zst: true,
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: empty_js_facts(id),
        };
        for desc in [
            u32_desc,
            pair_desc,
            fn_desc(FN_FLAT, 2, flat_id, flat_name),
            fn_desc(FN_VEC_PUSH, 3, push_id, push_name),
        ] {
            interns.types.insert(desc);
        }

        let place = |local: u32, ty: TypeId, proj: Vec<Projection>| Place {
            local: LocalId(local),
            projection: proj,
            ty,
        };
        let copy = |local: u32, ty: TypeId| Operand {
            ty,
            kind: OperandKind::Copy(place(local, ty, Vec::new())),
        };
        let u32_const = |v: u128| Operand {
            ty: U32,
            kind: OperandKind::Constant(Constant::Scalar { bits: v, bytes: 4 }),
        };
        let assign = |dest: u32, ty: TypeId, value: Rvalue| Statement {
            kind: StatementKind::Assign {
                destination: place(dest, ty, Vec::new()),
                value,
            },
        };

        // `vec_push$h3(x: u32) -> u32`: `CallSemantic::Standard(VecLen)` defers pre-repr
        // inlining so it only inlines during post-repr `o_group_with_mode(..., true)`.
        let vec_push_body = Body {
            item: push_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Standard(StandardOperation::VecLen),
            signature: Signature {
                arguments: vec![U32],
                result: U32,
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
            },
            locals: vec![
                Local {
                    id: LocalId(0),
                    role: LocalRole::Return,
                    ty: U32,
                },
                Local {
                    id: LocalId(1),
                    role: LocalRole::Argument,
                    ty: U32,
                },
            ],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: vec![assign(
                    0,
                    U32,
                    Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: copy(1, U32),
                        right: u32_const(1),
                    },
                )],
                terminator: Terminator::Return,
            }],
        };

        // `flat_callee$h2(p: (u32, u32)) -> u32`: big enough not to inline across 4 sites, small
        // enough for `abi_flat` to flatten `p` into two `u32` arguments.
        let mut flat_stmts = vec![
            assign(
                2,
                U32,
                Rvalue::Use(Operand {
                    ty: U32,
                    kind: OperandKind::Copy(place(
                        1,
                        U32,
                        vec![Projection::Field { field: 0, ty: U32 }],
                    )),
                }),
            ),
            assign(
                3,
                U32,
                Rvalue::Use(Operand {
                    ty: U32,
                    kind: OperandKind::Copy(place(
                        1,
                        U32,
                        vec![Projection::Field { field: 1, ty: U32 }],
                    )),
                }),
            ),
            assign(
                4,
                U32,
                Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: copy(2, U32),
                    right: copy(3, U32),
                },
            ),
        ];
        for step in 1..22u128 {
            flat_stmts.push(assign(
                4,
                U32,
                Rvalue::Binary {
                    op: BinaryOp::BitXor,
                    left: copy(4, U32),
                    right: u32_const(step),
                },
            ));
        }
        flat_stmts.push(assign(0, U32, Rvalue::Use(copy(4, U32))));
        let flat_body = Body {
            item: flat_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![PAIR],
                result: U32,
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    inline_hint: false,
                    inline_always: false,
                },
            },
            locals: vec![
                Local {
                    id: LocalId(0),
                    role: LocalRole::Return,
                    ty: U32,
                },
                Local {
                    id: LocalId(1),
                    role: LocalRole::Argument,
                    ty: PAIR,
                },
                Local {
                    id: LocalId(2),
                    role: LocalRole::Temporary,
                    ty: U32,
                },
                Local {
                    id: LocalId(3),
                    role: LocalRole::Temporary,
                    ty: U32,
                },
                Local {
                    id: LocalId(4),
                    role: LocalRole::Temporary,
                    ty: U32,
                },
            ],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: flat_stmts,
                terminator: Terminator::Return,
            }],
        };

        // `caller$h1(x: u32) -> u32`: calls `vec_push$h3(x)`, then calls `flat_callee$h2` 4 times
        // with freshly constructed `(u32, u32)` pairs so `abi_flat` accepts `flat_callee$h2`.
        let caller_body = {
            let mut blocks = Vec::new();
            blocks.push(BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Call(Call {
                    function: Operand {
                        ty: FN_VEC_PUSH,
                        kind: OperandKind::Constant(Constant::ZeroSized),
                    },
                    callee: Callee::Direct {
                        item: push_id,
                        link_name: push_name,
                    },
                    semantic: CallSemantic::Standard(StandardOperation::VecLen),
                    arguments: vec![copy(1, U32)],
                    destination: place(2, U32, Vec::new()),
                    target: Some(BlockId(1)),
                    unwind: Unwind::Continue,
                    track_caller: None,
                    host: None,
                }),
            });
            for idx in 0..4u32 {
                let pair_local = 3 + idx * 2;
                let res_local = 4 + idx * 2;
                blocks.push(BasicBlock {
                    id: BlockId(1 + idx),
                    cleanup: false,
                    statements: vec![assign(
                        pair_local,
                        PAIR,
                        Rvalue::Aggregate {
                            kind: AggregateKind::Tuple,
                            result: PAIR,
                            operands: vec![copy(2, U32), u32_const(u128::from(idx))],
                        },
                    )],
                    terminator: Terminator::Call(Call {
                        function: Operand {
                            ty: FN_FLAT,
                            kind: OperandKind::Constant(Constant::ZeroSized),
                        },
                        callee: Callee::Direct {
                            item: flat_id,
                            link_name: flat_name,
                        },
                        semantic: CallSemantic::Ordinary,
                        arguments: vec![copy(pair_local, PAIR)],
                        destination: place(res_local, U32, Vec::new()),
                        target: Some(BlockId(2 + idx)),
                        unwind: Unwind::Continue,
                        track_caller: None,
                        host: None,
                    }),
                });
            }
            blocks.push(BasicBlock {
                id: BlockId(5),
                cleanup: false,
                statements: vec![
                    assign(
                        11,
                        U32,
                        Rvalue::Binary {
                            op: BinaryOp::Add,
                            left: copy(4, U32),
                            right: copy(6, U32),
                        },
                    ),
                    assign(
                        12,
                        U32,
                        Rvalue::Binary {
                            op: BinaryOp::Add,
                            left: copy(8, U32),
                            right: copy(10, U32),
                        },
                    ),
                    assign(
                        0,
                        U32,
                        Rvalue::Binary {
                            op: BinaryOp::Add,
                            left: copy(11, U32),
                            right: copy(12, U32),
                        },
                    ),
                ],
                terminator: Terminator::Return,
            });
            let mut locals = vec![
                Local {
                    id: LocalId(0),
                    role: LocalRole::Return,
                    ty: U32,
                },
                Local {
                    id: LocalId(1),
                    role: LocalRole::Argument,
                    ty: U32,
                },
                Local {
                    id: LocalId(2),
                    role: LocalRole::Temporary,
                    ty: U32,
                },
            ];
            for idx in 0..4u32 {
                locals.push(Local {
                    id: LocalId(3 + idx * 2),
                    role: LocalRole::Temporary,
                    ty: PAIR,
                });
                locals.push(Local {
                    id: LocalId(4 + idx * 2),
                    role: LocalRole::Temporary,
                    ty: U32,
                });
            }
            locals.push(Local {
                id: LocalId(11),
                role: LocalRole::Temporary,
                ty: U32,
            });
            locals.push(Local {
                id: LocalId(12),
                role: LocalRole::Temporary,
                ty: U32,
            });
            Body {
                item: fn_item(1),
                kind: BodyKind::Function,
                semantic: BodySemantic::Ordinary,
                signature: Signature {
                    arguments: vec![U32],
                    result: U32,
                    abi: AbiSignature {
                        parameters: 1,
                        spread: None,
                        caller_location: false,
                        inline_never: false,
                        inline_hint: false,
                        inline_always: false,
                    },
                },
                locals,
                blocks,
            }
        };

        let body_origins = |item_path, body: &Body| {
            let stmt_count: usize = body.blocks.iter().map(|b| b.statements.len()).sum();
            let make_origin = |kind| Origin {
                kind,
                source: None,
                local_name: None,
                inline_stack: Vec::new(),
            };
            OriginTable {
                origins: vec![
                    make_origin(OriginKind::Statement),
                    make_origin(OriginKind::Terminator),
                    make_origin(OriginKind::Local),
                ],
                statements: vec![OriginId(0); stmt_count],
                terminators: vec![OriginId(1); body.blocks.len()],
                locals: vec![OriginId(2); body.locals.len()],
                item_path,
                root: None,
            }
        };
        let stub_decl = |name: &str| {
            jsast::function(name.to_owned(), Vec::new(), vec![jsast::Stmt::Return(None)])
        };
        let caller_name_str = interns.intern_str("caller$h1");
        let caller_decl = jsast::function(
            "caller$h1".to_owned(),
            vec!["x".to_owned()],
            vec![jsast::Stmt::Return(Some(jsast::binary(
                jsast::BinOp::Add,
                jsast::call(
                    jsast::Expr::Ident("vec_push$h3".to_owned()),
                    vec![jsast::Expr::Ident("x".to_owned())],
                ),
                jsast::call(
                    jsast::Expr::Ident("flat_callee$h2".to_owned()),
                    vec![jsast::Expr::Array(vec![])],
                ),
            )))],
        );

        let caller_origins = body_origins(caller_name_str, &caller_body);
        let flat_origins = body_origins(flat_name, &flat_body);
        let push_origins = body_origins(push_name, &vec_push_body);

        let js_items = vec![
            JsItem::new(
                JsName::new("caller$h1".to_owned()),
                ItemKind::Fn,
                caller_decl,
                Vec::new(),
                Linkage::exported("caller$h1".to_owned()),
                "test::caller$h1".to_owned(),
            )
            .with_jir(
                Some(JirItem {
                    body: caller_body,
                    origins: caller_origins,
                }),
                None,
            ),
            JsItem::new(
                JsName::new("flat_callee$h2".to_owned()),
                ItemKind::Fn,
                stub_decl("flat_callee$h2"),
                Vec::new(),
                Linkage::internal(),
                "test::flat_callee$h2".to_owned(),
            )
            .with_jir(
                Some(JirItem {
                    body: flat_body,
                    origins: flat_origins,
                }),
                None,
            ),
            JsItem::new(
                JsName::new("vec_push$h3".to_owned()),
                ItemKind::Fn,
                stub_decl("vec_push$h3"),
                Vec::new(),
                Linkage::internal(),
                "test::vec_push$h3".to_owned(),
            )
            .with_jir(
                Some(JirItem {
                    body: vec_push_body,
                    origins: push_origins,
                }),
                None,
            ),
        ];

        let obj_bytes = write_object(
            "",
            &js_items,
            true,
            "test",
            &ObjectIdentity::default(),
            interns,
            &BTreeMap::new(),
        );
        let read = read_object(&obj_bytes)
            .expect("valid object")
            .expect("format 20 object");
        let mut pristine_table = BTreeMap::new();
        for mut entry in read.items {
            assert!(entry.jir.decode().expect("decode jir"));
            pristine_table.insert(entry.name.clone(), entry);
        }

        let reachable = vec![
            JsName::new("caller$h1".to_owned()),
            JsName::new("flat_callee$h2".to_owned()),
            JsName::new("vec_push$h3".to_owned()),
        ];
        let sources = Default::default();
        let dcx = Sink::default();

        let opts_on = Opts {
            optimizer: OptimizerMode::On,
            lane: Lane::Jir,
            flat_abi: true,
            ..Opts::default()
        };
        let mut fault_table = pristine_table.clone();
        let fault_run = run_with_opts(
            &dcx,
            &mut fault_table,
            &reachable,
            &sources,
            &opts_on,
            Some("flat_callee"),
        );

        let opts_flat_off = Opts {
            optimizer: OptimizerMode::On,
            lane: Lane::Jir,
            flat_abi: false,
            ..Opts::default()
        };
        let mut off_table = pristine_table;
        let off_run = run_with_opts(
            &dcx,
            &mut off_table,
            &reachable,
            &sources,
            &opts_flat_off,
            None,
        );

        assert_eq!(
            fault_run.plan_retries, 1,
            "abi_json={:?}, passes={:?}, outcomes={:?}",
            fault_run.abi_json, fault_run.passes, fault_run.outcomes
        );
        assert_eq!(off_run.plan_retries, 0);
        assert!(
            fault_run.passes.applied.inline >= 1,
            "post-repr inline must inline vec_push after retry: passes={:?}, o={:?}, caller_js={}",
            fault_run.passes,
            fault_run.o,
            fault_table[&JsName::new("caller$h1".to_owned())].js
        );
        assert_eq!(
            fault_run.passes.applied.inline,
            off_run.passes.applied.inline
        );

        let caller_name = JsName::new("caller$h1".to_owned());
        assert!(
            !fault_table[&caller_name].js.contains("vec_push"),
            "vec_push must be inlined into caller on retry: {}",
            fault_table[&caller_name].js
        );
        for (name, off_item) in &off_table {
            assert_eq!(
                fault_table[name].js, off_item.js,
                "item {name} JS diverged between ABI-fault retry and flat_abi=off"
            );
        }
    }

    #[test]
    fn o_group_prunes_invoke_after_inlining_noop_wrapper_and_in_post_repr_mode() {
        use crate::jir::origin::{Origin, OriginKind, OriginTable};
        use crate::jir::semantic::{
            AbiSignature, BasicBlock, BlockId, Body, BodyKind, BodySemantic, Call, CallSemantic,
            Callee, DefinitionId, DirectItemRef, HostPayload, Local, LocalId, LocalRole, Operand,
            OperandKind, Place, Signature, Terminator, Unwind,
        };
        use crate::jir::types::{JsFacts, LayoutFacts, TypeDescriptor, TypeId, TypeKind, TypeMeta};
        use crate::jir::{Interns, OriginId};
        use crate::opts::{Lane, OptimizerMode, Opts};
        use rcgjs_object::item::{read_object, write_object, JirItem, JsItem, ObjectIdentity};

        const UNIT: TypeId = TypeId(1);
        const FN_EMPTY: TypeId = TypeId(2);
        const FN_WRAP: TypeId = TypeId(3);
        const FN_HOST: TypeId = TypeId(4);

        let fn_item = |local_hash: u64| ItemId {
            definition: DefinitionId {
                crate_id: 0,
                local_hash,
            },
            instantiation: u128::from(local_hash),
        };
        let caller_id = fn_item(1);
        let wrapper_id = fn_item(2);
        let empty_id = fn_item(3);

        let mut interns = Interns::new();
        let caller_str = interns.intern_str("caller$h1");
        let wrapper_str = interns.intern_str("wrapper$h2");
        let empty_str = interns.intern_str("empty_drop$h3");
        let host_sec = interns.intern_str("rcgjs.host.isnull");

        let empty_js_facts = |peeled: TypeId| JsFacts {
            int: None,
            is_indirect: false,
            needs_clone: false,
            crosses_transparent_union: false,
            transparent: None,
            peeled,
            unsized_tail: None,
            enum_repr: None,
            is_primitive_enum: false,
            pointer_form: None,
            byte_element: false,
            fields: Vec::new(),
        };
        interns.types.insert(TypeDescriptor {
            id: UNIT,
            name: None,
            kind: TypeKind::Tuple(Vec::new()),
            layout: LayoutFacts {
                size: Some(0),
                zst: true,
                sized: true,
                ..LayoutFacts::default()
            },
            meta: TypeMeta::default(),
            js: empty_js_facts(UNIT),
        });
        for (ty_id, h, item_id, name) in [
            (FN_EMPTY, 3, empty_id, empty_str),
            (FN_WRAP, 2, wrapper_id, wrapper_str),
            (
                FN_HOST,
                4,
                fn_item(4),
                interns.intern_str("isnull_marker$h4"),
            ),
        ] {
            interns.types.insert(TypeDescriptor {
                id: ty_id,
                name: None,
                kind: TypeKind::FnDef {
                    def: DefinitionId {
                        crate_id: 0,
                        local_hash: h,
                    },
                    sig: crate::jir::interns::SigId(0),
                    instance: Some(DirectItemRef {
                        item: item_id,
                        link_name: name,
                    }),
                },
                layout: LayoutFacts {
                    size: Some(0),
                    zst: true,
                    sized: true,
                    ..LayoutFacts::default()
                },
                meta: TypeMeta::default(),
                js: empty_js_facts(ty_id),
            });
        }

        let unit_sig = Signature {
            arguments: Vec::new(),
            result: UNIT,
            abi: AbiSignature::default(),
        };
        let unit_ret_local = Local {
            id: LocalId(0),
            role: LocalRole::Return,
            ty: UNIT,
        };
        let unit_place = Place {
            local: LocalId(0),
            projection: Vec::new(),
            ty: UNIT,
        };

        // `empty_drop$h3() { return; }`
        let empty_body = Body {
            item: empty_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: unit_sig.clone(),
            locals: vec![unit_ret_local],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements: Vec::new(),
                terminator: Terminator::Return,
            }],
        };

        // `wrapper$h2() { empty_drop$h3(); return; }` — non-empty before `inline`, becomes
        // 0-op `Return(None)` after `inline` splices `empty_drop$h3`.
        let wrapper_body = Body {
            item: wrapper_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: unit_sig.clone(),
            locals: vec![unit_ret_local],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Call(Call {
                        function: Operand {
                            ty: FN_EMPTY,
                            kind: OperandKind::Constant(crate::jir::semantic::Constant::ZeroSized),
                        },
                        callee: Callee::Direct {
                            item: empty_id,
                            link_name: empty_str,
                        },
                        semantic: CallSemantic::Ordinary,
                        arguments: Vec::new(),
                        destination: unit_place.clone(),
                        target: Some(BlockId(1)),
                        unwind: Unwind::Continue,
                        track_caller: None,
                        host: None,
                    }),
                },
                BasicBlock {
                    id: BlockId(1),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
            ],
        };

        // `caller$h1()` invokes a throwing host marker; its unwind cleanup calls `wrapper$h2` and
        // rethrows (`UnwindResume`). Only after `inline` empties `wrapper$h2` does post-inline
        // `prune_invoke::run` discover `wrapper$h2` is a noop and prune `caller`'s `Invoke`.
        let caller_body = Body {
            item: caller_id,
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: unit_sig,
            locals: vec![unit_ret_local],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Call(Call {
                        function: Operand {
                            ty: FN_HOST,
                            kind: OperandKind::Constant(crate::jir::semantic::Constant::ZeroSized),
                        },
                        callee: Callee::Direct {
                            item: fn_item(4),
                            link_name: empty_str,
                        },
                        semantic: CallSemantic::Host,
                        arguments: Vec::new(),
                        destination: unit_place.clone(),
                        target: Some(BlockId(1)),
                        unwind: Unwind::Cleanup(BlockId(2)),
                        track_caller: None,
                        host: Some(HostPayload::Marker {
                            section: host_sec,
                            def: DefinitionId {
                                crate_id: 0,
                                local_hash: 4,
                            },
                            template: None,
                            helpers: Vec::new(),
                        }),
                    }),
                },
                BasicBlock {
                    id: BlockId(1),
                    cleanup: false,
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
                BasicBlock {
                    id: BlockId(2),
                    cleanup: true,
                    statements: Vec::new(),
                    terminator: Terminator::Call(Call {
                        function: Operand {
                            ty: FN_WRAP,
                            kind: OperandKind::Constant(crate::jir::semantic::Constant::ZeroSized),
                        },
                        callee: Callee::Direct {
                            item: wrapper_id,
                            link_name: wrapper_str,
                        },
                        semantic: CallSemantic::Ordinary,
                        arguments: Vec::new(),
                        destination: unit_place,
                        target: Some(BlockId(3)),
                        unwind: Unwind::Terminate(crate::jir::semantic::TerminateReason::InCleanup),
                        track_caller: None,
                        host: None,
                    }),
                },
                BasicBlock {
                    id: BlockId(3),
                    cleanup: true,
                    statements: Vec::new(),
                    terminator: Terminator::UnwindResume,
                },
            ],
        };

        let make_origins = |item_path, body: &Body| OriginTable {
            origins: vec![
                Origin {
                    kind: OriginKind::Terminator,
                    source: None,
                    local_name: None,
                    inline_stack: Vec::new(),
                },
                Origin {
                    kind: OriginKind::Local,
                    source: None,
                    local_name: None,
                    inline_stack: Vec::new(),
                },
            ],
            statements: Vec::new(),
            terminators: vec![OriginId(0); body.blocks.len()],
            locals: vec![OriginId(1); body.locals.len()],
            item_path,
            root: None,
        };
        let stub_decl = |name: &str| {
            jsast::function(name.to_owned(), Vec::new(), vec![jsast::Stmt::Return(None)])
        };
        let caller_decl = jsast::function(
            "caller$h1".to_owned(),
            Vec::new(),
            vec![jsast::Stmt::Return(Some(jsast::call(
                jsast::Expr::Ident("wrapper$h2".to_owned()),
                Vec::new(),
            )))],
        );
        let wrapper_decl = jsast::function(
            "wrapper$h2".to_owned(),
            Vec::new(),
            vec![jsast::Stmt::Return(Some(jsast::call(
                jsast::Expr::Ident("empty_drop$h3".to_owned()),
                Vec::new(),
            )))],
        );
        let caller_origins = make_origins(caller_str, &caller_body);
        let wrapper_origins = make_origins(wrapper_str, &wrapper_body);
        let empty_origins = make_origins(empty_str, &empty_body);

        let js_items = vec![
            JsItem::new(
                JsName::new("caller$h1".to_owned()),
                ItemKind::Fn,
                caller_decl,
                Vec::new(),
                Linkage::exported("caller$h1".to_owned()),
                "test::caller$h1".to_owned(),
            )
            .with_jir(
                Some(JirItem {
                    body: caller_body,
                    origins: caller_origins,
                }),
                None,
            ),
            JsItem::new(
                JsName::new("wrapper$h2".to_owned()),
                ItemKind::Fn,
                wrapper_decl,
                Vec::new(),
                Linkage::internal(),
                "test::wrapper$h2".to_owned(),
            )
            .with_jir(
                Some(JirItem {
                    body: wrapper_body,
                    origins: wrapper_origins,
                }),
                None,
            ),
            JsItem::new(
                JsName::new("empty_drop$h3".to_owned()),
                ItemKind::Fn,
                stub_decl("empty_drop$h3"),
                Vec::new(),
                Linkage::internal(),
                "test::empty_drop$h3".to_owned(),
            )
            .with_jir(
                Some(JirItem {
                    body: empty_body,
                    origins: empty_origins,
                }),
                None,
            ),
        ];
        let obj_bytes = write_object(
            "",
            &js_items,
            true,
            "test",
            &ObjectIdentity::default(),
            interns,
            &BTreeMap::new(),
        );
        let read = read_object(&obj_bytes).unwrap().unwrap();
        let mut table = BTreeMap::new();
        for mut entry in read.items {
            assert!(entry.jir.decode().unwrap());
            table.insert(entry.name.clone(), entry);
        }

        let opts = Opts {
            optimizer: OptimizerMode::On,
            lane: Lane::Jir,
            ..Opts::default()
        };
        let (items, drop_glue) = item_index(&table);
        let vtables = vtable_index(&table);
        let ctx = Ctx {
            opts: &opts,
            items: &items,
            drop_glue: &drop_glue,
            vtables: &vtables,
            byte_views: crate::analysis::byte_views::ByteViewPlan::default(),
            heap: crate::analysis::heap::HeapPlan::default(),
        };
        let names = vec![
            JsName::new("caller$h1".to_owned()),
            JsName::new("wrapper$h2".to_owned()),
            JsName::new("empty_drop$h3".to_owned()),
        ];
        let live: BTreeSet<JsName> = names.iter().cloned().collect();
        let mut funcs = build_funcs(&table, &names, &ctx);
        assert!(funcs[&JsName::new("caller$h1".to_owned())]
            .as_ref()
            .unwrap()
            .has_invoke());

        let group = o_group_with_mode(&mut table, &mut funcs, &ctx, &live, true, None);
        assert_eq!(group.inline.applied, 1);
        assert!(
            !funcs[&JsName::new("caller$h1".to_owned())]
                .as_ref()
                .unwrap()
                .has_invoke(),
            "post-inline prune_invoke must prune Invoke once inline empties wrapper$h2 (even with allow_std_post_repr=true)"
        );
    }
}
