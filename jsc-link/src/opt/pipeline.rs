//! The pass pipeline: which passes run, in what order, and what a failed one costs.
//!
//! Every pass is followed by [`Func::verify`]. A verify failure is a [`Decline`] — the item falls
//! back to the baseline text and the link still produces a program — and never a panic: a pass
//! that miscompiles one function must not take the build down with it.
//!
//! The order is `dce` and the escape analysis alternating to a short fixpoint, then `range`,
//! then one cleanup `dce`. Running `dce` first matters for the discriminant jump-threading: a
//! merge block split into per-variant edges *before* the escape analysis runs is what lets it
//! scalarize an enum that would otherwise escape through the merge, and a scalarized outer
//! object exposes its inner one to the next round. The `dce` after the escape analysis is the
//! original cleanup: the optimistic per-field parameters are deleted where nothing varies.
//! `range` runs late because its witnesses name blocks and values of the function it saw, and it
//! only removes `Assert` ops — it never renumbers; the trailing `dce` then sweeps the dead
//! comparisons and length reads the removed asserts leave, with witnesses of its own.
//!
//! `RCGJS_JIR_PASSES` (link-time, D5) selects a subset of the passes by name for the metamorphic
//! matrix: a comma-separated list out of
//! `constprop,inline,devirt,pea,host-args,slice-iter,range,dce`; unset runs them all. `dce` maps to
//! the deletions and the discriminant threading, `constprop` to the constant-value parameter merge;
//! the constant folds themselves are `dce`'s always-on floor (see [`dce::Options`]) — without them
//! a lane item would reference panics the producer-baked baseline folded away before its references
//! were taken, and the link would refuse it. Names no pass here consumes yet (`inline`, `devirt`)
//! are accepted and ignored.
//!
//! Every class-named rewrite records a [`Witness`], and the group ends with the independent
//! [`witness::check_all`] in debug builds and under `RCGJS_WITNESS_CHECK=1`. A failed check is a
//! `proof.witness` decline: the item falls back, the link goes on. `RCGJS_WITNESS_FAULT=1`
//! corrupts every recorded witness before the check. `RCGJS_WITNESS_FAULT=slice-iterator`
//! corrupts only the exact slice-cursor witness.

use std::sync::OnceLock;

use crate::jir::decline::{Decline, LowerDecline, OpClass, ProofDecline};
use crate::jir::repr::Func;
use crate::jir::TypeTable;
use crate::opts::{OptimizerPass, Opts};

use super::witness::{self, Witness};
use super::{dce, partial_escape::PeaStats, range};

/// What the passes did to one function. The driver sums these for the sidecar's `passes` block.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OptStats {
    pub host_args: super::host_args::HostArgsStats,
    /// Whether `sroa` (the partial escape analysis) changed the function.
    pub sroa_changed: bool,
    /// Whether `ast-dce` changed the function.
    pub ast_dce_changed: bool,
    /// Whether the range pass removed anything.
    pub range_changed: bool,
    /// Bounds-check asserts the range pass removed.
    pub asserts_removed: u32,
    /// Aligned slice-prefix comparisons the range pass folded.
    pub range_comparisons_folded: u32,
    /// Exact standard-library slice-adapter loops that would lower to checked cursors.
    pub slice_iterator_loops_would_lower: u32,
    /// Witnesses the passes recorded.
    pub witnesses_recorded: u32,
    /// Witnesses that failed their independent check (the item declines when any does).
    pub witnesses_failed: u32,
    /// What the escape analysis saw.
    pub pea: PeaStats,
}

/// The pass subset `RCGJS_JIR_PASSES` selects; every field true when the env is unset.
struct PassSet {
    constprop: bool,
    pea: bool,
    dce: bool,
    slice_iter: bool,
    range: bool,
    inline: bool,
    devirt: bool,
    host_args: bool,
}

impl PassSet {
    const ALL: Self = Self {
        constprop: true,
        pea: true,
        dce: true,
        slice_iter: true,
        range: true,
        inline: true,
        devirt: true,
        host_args: true,
    };
}

/// Reads `RCGJS_JIR_PASSES` once per process: the env cannot change mid-link, and reading it per
/// function would let it.
fn pass_set() -> &'static PassSet {
    static SET: OnceLock<PassSet> = OnceLock::new();
    SET.get_or_init(|| match std::env::var("RCGJS_JIR_PASSES") {
        Err(_) => PassSet::ALL,
        Ok(names) => {
            let has = |name: &str| names.split(',').any(|part| part.trim() == name);
            PassSet {
                constprop: has("constprop"),
                pea: has("pea"),
                dce: has("dce"),
                slice_iter: has("slice-iter"),
                range: has("range"),
                inline: has("inline"),
                devirt: has("devirt"),
                host_args: has("host-args"),
            }
        }
    })
}

/// Whether the O group's pass `name` (`inline`, `devirt`, `dce`, `constprop`) is selected by
/// `RCGJS_JIR_PASSES` — the lane driver's half of the same switch the per-item passes read.
pub fn o_pass_enabled(name: &str) -> bool {
    let passes = pass_set();
    match name {
        "inline" => passes.inline,
        "devirt" => passes.devirt,
        "dce" => passes.dce,
        "constprop" => passes.constprop,
        _ => false,
    }
}

/// Runs the lane's optimizer over one function, in place.
///
/// Each pass runs only when `opts.analyzes` its name, and is verified only when it changed
/// something: an unchanged function was verified by the SSA builder already.
///
/// Under `js-opt=report` no pass *applies*, and the program the link step prints has to be the
/// one `js-opt=off` prints, byte for byte. So the passes run over a clone, their stats are kept
/// and the rewritten clone is dropped — including when it fails to verify, which is reported as
/// "would not have applied" rather than declining the item, since a decline would be a
/// difference `off` does not have.
pub fn optimize(
    func: &mut Func,
    table: &TypeTable,
    interns: &crate::jir::interns::Interns,
    opts: &Opts,
    stats: &mut OptStats,
) -> Result<(), Decline> {
    if reports(opts, OptimizerPass::Sroa) || reports(opts, OptimizerPass::AstDce) {
        let mut probe = func.clone();
        if run(&mut probe, table, interns, opts, stats).is_err() {
            *stats = OptStats::default();
        }
        return Ok(());
    }
    run(func, table, interns, opts, stats)
}

/// Whether `pass` is one this mode measures without keeping: `js-opt=report`'s whole job.
fn reports(opts: &Opts, pass: OptimizerPass) -> bool {
    opts.analyzes(pass) && !opts.applies(pass)
}

/// The passes themselves, over whichever function the caller is willing to have rewritten.
fn run(
    func: &mut Func,
    table: &TypeTable,
    interns: &crate::jir::interns::Interns,
    opts: &Opts,
    stats: &mut OptStats,
) -> Result<(), Decline> {
    let passes = pass_set();
    let mut witnesses: Vec<Witness> = Vec::new();
    let dce_options = dce::Options {
        no_constant_params: !passes.constprop,
        no_deletions: !passes.dce,
        pure_call: None,
    };
    let runs_dce = opts.analyzes(OptimizerPass::AstDce);
    if runs_dce && passes.dce && passes.host_args {
        stats.host_args = super::host_args::run(func, table, interns);
        if stats.host_args.would_lower > 0 {
            verified(func, table, "host-arguments")?;
            stats.ast_dce_changed = true;
        }
    }

    // PEA keeps admitted native roots opaque and preserves their copies, borrows and conversions
    // while virtualizing unrelated baseline products. Field shapes and reconstructed edges are
    // checked independently. Other carrier families still need executable composition coverage
    // before admission, including their storage and ownership contracts.
    let pea_carriers = func.values.iter().all(|def| {
        matches!(
            def.carrier,
            crate::jir::repr::Carrier::Baseline
                | crate::jir::repr::Carrier::HostString
                | crate::jir::repr::Carrier::SliceView
                | crate::jir::repr::Carrier::StringBuilder
                | crate::jir::repr::Carrier::NativeVec
                | crate::jir::repr::Carrier::NativeBox
        )
    });
    let runs_pea = opts.analyzes(OptimizerPass::Sroa) && passes.pea && pea_carriers;

    // `dce` then the escape analysis, to a short fixpoint: the first `dce` threads the
    // variant-per-path merges the escape analysis then scalarizes, and a scalarized outer object
    // exposes its inner one to the next round (a `Some((a, b))` merge takes two). The trailing
    // `dce` of the last round is the cleanup the escape analysis counts on: its optimistic
    // per-field parameters are deleted wherever nothing varies.
    for _ in 0..3 {
        let mut round = false;
        if runs_dce {
            let changed = dce::run_with(func, table, &dce_options, &mut witnesses);
            stats.ast_dce_changed |= changed;
            round |= changed;
            if changed {
                verified(func, table, "ast-dce")?;
            }
        }
        if runs_pea {
            let pea = super::partial_escape::run(func, table);
            stats.pea.objects += pea.objects;
            stats.pea.scalarized += pea.scalarized;
            stats.pea.escaped += pea.escaped;
            stats.pea.params_added += pea.params_added;
            if pea.scalarized > 0 {
                stats.sroa_changed = true;
                round = true;
                witnesses.push(Witness::Scalarize {
                    scalarized: pea.scalarized,
                });
                verified(func, table, "sroa")?;
            }
        }
        if !round {
            break;
        }
    }
    if stats.sroa_changed && runs_dce {
        let changed = dce::run_with(func, table, &dce_options, &mut witnesses);
        stats.ast_dce_changed |= changed;
        if changed {
            verified(func, table, "ast-dce")?;
        }
    }

    if runs_dce && passes.slice_iter {
        let lowered = super::slice_iter::run(func, table);
        if !lowered.is_empty() {
            stats.slice_iterator_loops_would_lower += lowered.len() as u32;
            let mut slice_witnesses = [Witness::SliceIteratorCursor { loops: lowered }];

            // These witnesses carry exact block and value ids. Check them before DCE removes the
            // old adapter graph and renumbers the generated loop. DCE then verifies its own
            // output and records a separate payload-only witness batch.
            checked(&mut slice_witnesses, func, table, stats)?;
            if dce::run_with(func, table, &dce_options, &mut witnesses) {
                stats.ast_dce_changed = true;
            }
            verified(func, table, "slice-iterator-cursor")?;
        }
    }

    if runs_dce {
        let forwarded = super::slice_forward::run(func, table);
        if forwarded > 0 {
            witnesses.push(Witness::SliceViewForward { views: forwarded });
            verified(func, table, "slice-forward")?;
            if dce::run_with(func, table, &dce_options, &mut witnesses) {
                stats.ast_dce_changed = true;
                verified(func, table, "ast-dce")?;
            }
        }
    }

    if opts.analyzes(OptimizerPass::AstDce) && passes.range {
        let range = range::run(func, table, &mut witnesses);
        if range.changed() {
            stats.range_changed = true;
            stats.asserts_removed += range.asserts_removed;
            stats.range_comparisons_folded += range.comparisons_folded;
            verified(func, table, "range")?;
        }
    }

    // The `BoundsRemoved` witnesses name blocks and values of the function as `range` left it,
    // so they are checked before the cleanup below renumbers anything.
    checked(&mut witnesses, func, table, stats)?;

    // The removed asserts leave their comparisons and length reads dead; this cleanup's own
    // witnesses are payload-only and get their own check.
    if stats.range_changed && runs_dce {
        let mut cleanup: Vec<Witness> = Vec::new();
        if dce::run_with(func, table, &dce_options, &mut cleanup) {
            stats.ast_dce_changed = true;
            verified(func, table, "ast-dce")?;
        }
        checked(&mut cleanup, func, table, stats)?;
    }
    Ok(())
}

/// Runs the independent witness checks over one batch, when they are enabled, and turns a
/// failure into the `proof.witness` decline.
fn checked(
    witnesses: &mut [Witness],
    func: &Func,
    table: &TypeTable,
    stats: &mut OptStats,
) -> Result<(), Decline> {
    stats.witnesses_recorded += witnesses.len() as u32;
    if !witness::enabled() || witnesses.is_empty() {
        return Ok(());
    }
    if let Some(target) = std::env::var_os("RCGJS_WITNESS_FAULT") {
        corrupt(witnesses, target.to_string_lossy().as_ref());
    }
    if witness::check_all(witnesses, func, table).is_err() {
        stats.witnesses_failed += 1;
        return Err(Decline::Proof(ProofDecline::Witness));
    }
    Ok(())
}

/// Falsifies all witnesses, or only the named witness class, for a transactional fallback test.
fn corrupt(witnesses: &mut [Witness], target: &str) {
    for witness in witnesses.iter_mut() {
        if target == "slice-iterator" && !matches!(witness, Witness::SliceIteratorCursor { .. }) {
            continue;
        }
        match witness {
            Witness::ConstFold { result, .. } => *result = !*result,
            Witness::BranchFold { taken, .. } => *taken = u32::MAX,
            Witness::BoundsRemoved { fact, .. } => {
                *fact = super::witness::BoundsFact::Numeric {
                    index_hi: 1,
                    length_lo: 0,
                }
            }
            Witness::AlignedPrefix { result, .. } => *result = !*result,
            Witness::Inline {
                bytes_after, limit, ..
            } => *bytes_after = *limit + 1,
            Witness::Scalarize { .. }
            | Witness::Adapter { .. }
            | Witness::DropElided { .. }
            | Witness::Devirt { .. } => {}
            Witness::UnsignedAddOverflowSplit { bits } => *bits = 0,
            Witness::ReturnProjection { fields } => *fields = 0,
            Witness::SliceViewForward { views } => *views = 0,
            Witness::SliceIteratorCursor { loops } => {
                if let Some(loop_) = loops.first_mut() {
                    loop_.header = crate::jir::repr::RBlockId(u32::MAX);
                } else {
                    loops.clear();
                }
            }
        }
    }
}

/// `Func::verify` as a [`Decline`]: the item falls back rather than the build failing.
///
/// Verified under the carriers the function's values already have: a committed representation
/// plan (Phase 4) leaves non-baseline carriers behind, which its own commit verified under the
/// exact committed policy — a pass must not decline the item for carrying them. A pass cannot
/// invent a carrier (it only moves and copies `ValueDef`s), so deriving the policy from the
/// values keeps the canary while accepting the plan's work.
fn verified(func: &Func, table: &TypeTable, pass: &str) -> Result<(), Decline> {
    func.verify_carriers(table, observed_carriers(func))
        .map_err(|err| {
            Decline::Lower(LowerDecline::Other {
                op: OpClass::Region,
                detail: format!("opt/{pass}: {err}"),
            })
        })
}

/// The carriers a function's values actually use: the policy an already-committed function is
/// re-verified under after a pass or a plan rewrite.
pub fn observed_carriers(func: &Func) -> crate::jir::repr::CarrierPolicy {
    func.values
        .iter()
        .fold(crate::jir::repr::CarrierPolicy::BASELINE, |policy, def| {
            policy.with(def.carrier)
        })
}
