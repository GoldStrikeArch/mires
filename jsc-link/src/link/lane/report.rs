//! Sidecar accounting and JSON serialization for one lane run (`<out>.lane.json`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::diag::Sink;
use crate::item::JsName;
use crate::jir::decline::Decline;
use crate::jir::summary::ItemClass;
use crate::lane_report::{push_string, ItemLane};
use crate::opts::Lane;

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

/// Whether the JIR lowering succeeded or fell back with a decline reason.
#[derive(Clone, Debug)]
pub(crate) enum LaneDisposition {
    /// Lowered and installed on the JIR lane.
    Jir {
        /// The bytes the JIR lowering printed.
        bytes_jir: usize,
        /// The bytes the item takes printed compact (`jsast::Style::Compact`) with its locals
        /// shortened (`minify::shorten_locals_for_measurement`), always measured.
        ///
        /// A size claim about ONE item otherwise needs `js-minify` on the whole build, which
        /// changes every other item too. The shortening is done on a clone and is never what a
        /// build prints.
        bytes_min: usize,
        /// Whether the lowering reaches exactly what the baseline reached — the linkable subset of
        /// `refs`. `false` means the lowering proved something dead (or spells a prepared name
        /// the baseline spelled another way), and the link step walks reachability again
        /// ([`LaneRun::references_changed`]).
        refs_equal: bool,
    },
    /// Declined during lowering or refused during installation.
    Fallback {
        decline: Decline,
        /// `false` on a refused lowering whose refs disagreed; `true` when lowering declined before
        /// installation.
        refs_equal: bool,
    },
}

/// What the driver did with one item it offered to the lowerer.
#[derive(Clone, Debug)]
pub(crate) struct LaneOutcome {
    pub name: JsName,
    pub class: ItemClass,
    pub disposition: LaneDisposition,
    pub bytes_baseline: usize,
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

impl LaneOutcome {
    pub(crate) fn lane(&self) -> ItemLane {
        match self.disposition {
            LaneDisposition::Jir { .. } => ItemLane::Jir,
            LaneDisposition::Fallback { .. } => ItemLane::Fallback,
        }
    }

    pub(crate) fn is_jir(&self) -> bool {
        matches!(self.disposition, LaneDisposition::Jir { .. })
    }

    pub(crate) fn decline(&self) -> Option<&Decline> {
        match &self.disposition {
            LaneDisposition::Jir { .. } => None,
            LaneDisposition::Fallback { decline, .. } => Some(decline),
        }
    }

    pub(crate) fn refs_equal(&self) -> bool {
        match self.disposition {
            LaneDisposition::Jir { refs_equal, .. }
            | LaneDisposition::Fallback { refs_equal, .. } => refs_equal,
        }
    }

    pub(crate) fn bytes_jir(&self) -> usize {
        match self.disposition {
            LaneDisposition::Jir { bytes_jir, .. } => bytes_jir,
            LaneDisposition::Fallback { .. } => 0,
        }
    }

    pub(crate) fn bytes_min(&self) -> usize {
        match self.disposition {
            LaneDisposition::Jir { bytes_min, .. } => bytes_min,
            LaneDisposition::Fallback { .. } => 0,
        }
    }
}

/// How many items each pass changed. One half of the sidecar's `passes` block.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PassCounts {
    pub(super) sroa: usize,
    pub(super) ast_dce: usize,
    pub(super) inline: usize,
    pub(super) devirt: usize,
}

/// The sidecar's `passes` block: what the passes *would* do and what they did.
///
/// The two are equal under `js-opt=on`. Under `js-opt=report` the passes run over a clone of each
/// function and the rewrite is thrown away (`opt::optimize`), so `would_apply` is what `on` would
/// have changed and `applied` is all zero — the program is the one `js-opt=off` prints. Under
/// `js-opt=off` nothing runs and both are zero.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Passes {
    pub(super) would_apply: PassCounts,
    pub(super) applied: PassCounts,
}

/// The O group's whole-map accounting (Phase 5, D5): what the interprocedural passes did — or,
/// under `js-opt=report`, would have done on the clone, for the sidecar's optimizer blocks.
#[derive(Clone, Debug, Default)]
pub(crate) struct OGroup {
    /// How many bodies got an effects summary, how many the post-group revalidation confirmed,
    /// and how many sit at top (unusable) after it.
    pub(super) summaries_items: u32,
    pub(super) summaries_validated: u32,
    pub(super) summaries_top: u32,
    pub(super) inline: crate::opt::inline::InlineStats,
    pub(super) devirt: crate::opt::devirtualize::DevirtStats,
    /// The items the group's own dce run changed (the pure-call deletions ride it).
    pub(super) dce_items: BTreeSet<JsName>,
    /// Witnesses the group's dce runs recorded and failed (inline/devirt carry their own).
    pub(super) witnesses_recorded: u32,
    pub(super) witnesses_failed: u32,
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
    pub(super) outcomes: BTreeMap<JsName, LaneOutcome>,
    pub(super) passes: Passes,
    /// The O group's accounting; all zero when the group did not run.
    pub(super) o: OGroup,
    /// Bounds-check asserts the range pass removed, summed over the items.
    pub(super) range_asserts: u64,
    /// Aligned slice-prefix comparisons the range pass folded, summed over the items.
    pub(super) range_comparisons: u64,
    /// Witnesses recorded and failed, the per-item passes and the O group together.
    pub(super) witnesses_recorded: u64,
    pub(super) witnesses_failed: u64,
    /// The flat-call ABI plan as JSON, when one was made (W3).
    pub(super) abi_json: Option<String>,
    /// The representation plan's accounting, when its analysis ran (W4). Empty until the solver
    /// lands; `None` renders as `"repr":null`.
    pub(super) repr: Option<ReprBlock>,
    /// How many times the run restarted with an empty ABI plan. See [`super::run`].
    pub(super) plan_retries: u32,
    /// Whether the lane added items to the table (the `$rN` spill registers), which is what makes
    /// the link step walk reachability again.
    pub(super) added_items: bool,
    /// Whole-program lazy byte-view policy.
    pub(super) byte_views: crate::analysis::byte_views::ByteViewPlan,
    /// Whether universal flat `Vec<T, Global>` representation was active.
    pub(super) native_vec_enabled: bool,
}

impl LaneRun {
    /// Whether universal flat `Vec<T, Global>` representation was active.
    pub(crate) fn native_vec_enabled(&self) -> bool {
        self.native_vec_enabled
    }

    /// The items the lowerer declined. The exit gate's number.
    pub(crate) fn fallback_count(&self) -> usize {
        self.outcomes
            .values()
            .filter(|outcome| !outcome.is_jir())
            .count()
    }

    /// The row for `name`, for the lane report's `lane` field.
    pub(crate) fn outcome(&self, name: &JsName) -> Option<&LaneOutcome> {
        self.outcomes.get(name)
    }

    /// Drops the rows of items that are no longer in the program.
    ///
    /// `link_dedup` runs after the lane and merges duplicates, so an item the lane lowered can
    /// stop being printed under its own name. The sidecar describes the program that came out.
    pub(crate) fn retain_reachable(&mut self, reachable: &[JsName]) {
        let live: BTreeSet<&JsName> = reachable.iter().collect();
        self.outcomes.retain(|name, _| live.contains(name));
        if let Some(repr) = &mut self.repr {
            repr.retain_reachable(&live);
        }
    }

    /// `js-str=bytes` moved the lowered body of an export to `to` and put a facade under its
    /// name (`link.rs::byte_string_facades`); the outcome follows the body.
    pub(crate) fn rename(&mut self, from: &JsName, to: &JsName) {
        if let Some(mut outcome) = self.outcomes.remove(from) {
            outcome.name = to.clone();
            self.outcomes.insert(to.clone(), outcome);
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
            .values()
            .any(|outcome| outcome.is_jir() && !outcome.refs_equal())
    }

    /// `<out>.lane.json`: what each offered item did, sorted by name.
    pub(crate) fn render(&self, lane: Lane) -> String {
        let mut out = String::with_capacity(128 * self.outcomes.len() + 128);
        let slice_iterator_would_lower: u64 = self
            .outcomes
            .values()
            .map(|outcome| u64::from(outcome.slice_iterator_would_lower))
            .sum();
        let slice_iterator_final_emitted: u64 = self
            .outcomes
            .values()
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
             \"plan_retries\":{},\"abi\":{},\"physical\":null,\"async\":null,\"repr\":",
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
        );
        match &self.repr {
            Some(repr) => repr.render(&mut out),
            None => out.push_str("null"),
        }
        out.push_str(",\"items\":[");
        for (index, outcome) in self.outcomes.values().enumerate() {
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
            push_string(&mut out, outcome.lane().as_str());
            out.push_str(",\"decline\":");
            match outcome.decline() {
                Some(decline) => push_string(&mut out, decline.code()),
                None => out.push_str("null"),
            }
            let _ = write!(
                out,
                ",\"refs_equal\":{},\"bytes_baseline\":{},\"bytes_jir\":{},\"bytes_min\":{},\
                 \"slice_iter\":{{\"would_lower\":{},\"final_emitted\":{}}},\
                 \"host_args\":{},\"alloc\":{},\"js_alloc\":{}}}",
                outcome.refs_equal(),
                outcome.bytes_baseline,
                outcome.bytes_jir(),
                outcome.bytes_min(),
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
