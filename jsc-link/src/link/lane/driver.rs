//! Multi-phase lane driver: Phase A (JIR-R build), Phase O, Phase B1/B2 planning, Phase C
//! transactional lowering with retry, and installation into the link item table.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::analysis::arena::ProgramBoundsFacts;
use crate::analysis::byte_views::ByteViewPlan;
use crate::analysis::heap::HeapPlan;
use crate::diag::Sink;
use crate::item::{item_map, prologue, write_program, JsItem, JsName, LinkItem};
use crate::jir::decline::{Decline, LowerDecline, OpClass};
use crate::jir::repr::Func;
use crate::jir::semantic::BodySemantic;
use crate::jsast;
use crate::lower::js::host::{DropGlueIndex, ItemEntry, ItemIndex};
use crate::lower::js::unsize::VtableIndex;
use crate::lower::{lower_item, LowerInput, LoweredItem};
use crate::opt::OptStats;
use crate::opts::{OptimizerMode, OptimizerPass, Opts, Profile};
use crate::plan::abi_flat::AbiPlan;
use crate::plan::enum_repr::EnumReprPlan;
use crate::plan::native_types::{NativeBody, ReprOutcome};
use crate::plan::soa_vec::SoaPlan;

use super::o_group::{merge_o_stats, o_group, o_group_with_mode};
use super::report::{LaneDisposition, LaneOutcome, LaneRun, OGroup, Passes};

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
/// * **C** runs [`crate::opt::optimize_with_facts`] over each `Func` and prints it
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
) -> LaneRun {
    let opts = crate::opts::get();
    let fault = abi_fault();
    run_with_opts(dcx, table, reachable, opts, fault.as_deref())
}

pub(super) fn run_with_opts(
    dcx: &Sink,
    table: &mut BTreeMap<JsName, LinkItem>,
    reachable: &[JsName],
    opts: &Opts,
    fault: Option<&str>,
) -> LaneRun {
    let _lane_scope = crate::profile::scope("lane.total");
    LaneDriver::new(dcx, table, reachable, opts, fault).execute()
}

/// Owned program-wide indexes and analyses held alongside [`LaneDriver`], borrowable as a [`Ctx`]
/// independently of `table`, `defined`, and `live`.
struct LaneIndexes<'a> {
    opts: &'a Opts,
    items: ItemIndex,
    drop_glue: DropGlueIndex,
    vtables: VtableIndex,
    byte_views: ByteViewPlan,
    heap: HeapPlan,
}

impl<'a> LaneIndexes<'a> {
    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            opts: self.opts,
            items: &self.items,
            drop_glue: &self.drop_glue,
            vtables: &self.vtables,
            byte_views: self.byte_views,
            heap: &self.heap,
        }
    }

    fn default_analysis_ctx(&self, default_heap: &'a HeapPlan) -> Ctx<'_> {
        Ctx {
            opts: self.opts,
            items: &self.items,
            drop_glue: &self.drop_glue,
            vtables: &self.vtables,
            byte_views: ByteViewPlan::default(),
            heap: default_heap,
        }
    }
}

/// Outputs of Phase A (enum specialization and initial JIR-R build).
struct StageA {
    unspecialized_table: Option<BTreeMap<JsName, LinkItem>>,
    enum_excluded: BTreeSet<rcgjs_object::jir::semantic::TypeId>,
    enum_plan: EnumReprPlan,
    pristine_funcs: BTreeMap<JsName, Result<Func, Decline>>,
    pristine_table: BTreeMap<JsName, LinkItem>,
}

/// Outputs of pre-representation Phase O, Phase B1 native types, and post-representation Phase O.
struct StageB1 {
    planned_funcs: BTreeMap<JsName, Result<Func, Decline>>,
    planning_table: BTreeMap<JsName, LinkItem>,
    funcs: BTreeMap<JsName, Result<Func, Decline>>,
    work_table: BTreeMap<JsName, LinkItem>,
    o_stats: OGroup,
    repr_ran: bool,
    repr: ReprOutcome,
    planned_body_names: Vec<JsName>,
    planned_has_native_map: bool,
    pre_repr_native_string: bool,
    pre_repr_native_vec: bool,
}

/// Outputs of Phase B2 physical ABI planning and pricing.
struct StageB2 {
    plan: AbiPlan,
    abi_applied: bool,
    abi_clone_names: BTreeSet<JsName>,
    reported: Option<String>,
}

/// Synthetic item names prepared before transactional Phase C lowering.
struct SyntheticNames {
    bstr_const_items: Vec<LinkItem>,
    bigint_names: Option<crate::lower::bigint::BigIntNames>,
}

/// Final Phase C lowering outputs passed to outcome commitment.
struct LoweringResult {
    lowered: Vec<Lowered>,
    final_soa_plan: Arc<SoaPlan>,
    plan_retries: u32,
}

/// Orchestrates all stages of one lane pass over `table`.
struct LaneDriver<'a> {
    dcx: &'a Sink,
    table: &'a mut BTreeMap<JsName, LinkItem>,
    reachable: &'a [JsName],
    opts: &'a Opts,
    fault: Option<&'a str>,
    defined: BTreeSet<JsName>,
    live: BTreeSet<JsName>,
    indexes: LaneIndexes<'a>,
    original_names: Vec<JsName>,
    names: Vec<JsName>,
}

impl<'a> LaneDriver<'a> {
    fn new(
        dcx: &'a Sink,
        table: &'a mut BTreeMap<JsName, LinkItem>,
        reachable: &'a [JsName],
        opts: &'a Opts,
        fault: Option<&'a str>,
    ) -> Self {
        // The names the reachability walk can follow, taken before any item is replaced. Cloned
        // rather than borrowed because the loop below holds `&mut` on one item of the same table.
        let defined: BTreeSet<JsName> = table.keys().cloned().collect();
        // What the walk has already decoded, ordered and will print. A lowering may reference any of
        // it, whether or not the baseline did; see `apply`.
        let live: BTreeSet<JsName> = reachable.iter().cloned().collect();
        // Every item of the program by its semantic identity: what the host lowering resolves a
        // closure body's name and arity through (`lower/js/host.rs`).
        let (items, drop_glue) = item_index(table);
        let vtables = vtable_index(table);
        let original_names = offered(table, reachable, opts);
        let names = original_names.clone();
        Self {
            dcx,
            table,
            reachable,
            opts,
            fault,
            defined,
            live,
            indexes: LaneIndexes {
                opts,
                items,
                drop_glue,
                vtables,
                byte_views: ByteViewPlan::default(),
                heap: HeapPlan::default(),
            },
            original_names,
            names,
        }
    }

    fn execute(mut self) -> LaneRun {
        let mut stage_a = self.specialize_enums_and_build_a();
        let mut stage_b1 = self.run_pre_repr_o_and_b1(&stage_a);
        let mut stage_b2 = self.plan_and_price_abi(&stage_a, &mut stage_b1);
        let synth = self.prepare_synthetic_names(&stage_b2.plan, &stage_b1.work_table);
        let mut result = self.lower_with_retry(&mut stage_a, &mut stage_b1, &mut stage_b2, &synth);
        let added_items = self.install_synthesized_items(
            &mut result.lowered,
            &stage_b2.plan,
            &stage_a.enum_plan,
            &stage_b2.abi_clone_names,
            synth,
        );
        self.commit_outcomes(result, &stage_b1, &stage_b2, added_items)
    }

    fn install_enum_sentinels(&mut self, enum_plan: &EnumReprPlan) {
        for sentinel in enum_plan.sentinel_items() {
            self.defined.insert(sentinel.name.clone());
            self.live.insert(sentinel.name.clone());
            self.table.insert(sentinel.name.clone(), sentinel);
        }
    }

    fn run_o_if_enabled(&mut self, funcs: &mut BTreeMap<JsName, Result<Func, Decline>>) -> OGroup {
        if self.opts.analyzes(OptimizerPass::AstDce) {
            let ctx = self.indexes.ctx();
            o_group(self.table, funcs, &ctx, &self.live)
        } else {
            OGroup::default()
        }
    }

    /// Stage 1: specialize enums (under `ReleaseSpeed`), build initial JIR-R `Func`s, retry enum
    /// exclusions on Phase A declines, and initialize byte-view / heap analyses.
    fn specialize_enums_and_build_a(&mut self) -> StageA {
        // Link-time whole-program enum specialization (`EnumRepr::NullNiche` and `EnumRepr::IntTag`)
        // runs BEFORE Phase A `build_funcs` under `ReleaseSpeed` so SSA cell boxing,
        // `SetDiscriminant`, `needs_clone`, `fresh_container`, and `PointerForm::Thin -> Slot` all
        // observe one consistent `TypeTable`. Skipped completely under `Report` / `Off` /
        // non-`ReleaseSpeed`.
        let mut enum_excluded: BTreeSet<rcgjs_object::jir::semantic::TypeId> = BTreeSet::new();
        let unspecialized_table = (self.opts.optimizer == OptimizerMode::On
            && self.opts.profile == Some(Profile::ReleaseSpeed))
        .then(|| self.table.clone());
        let mut enum_plan = crate::plan::enum_repr::specialize_table(
            self.table,
            self.reachable,
            &self.original_names,
            &enum_excluded,
            self.opts,
        );
        self.install_enum_sentinels(&enum_plan);

        // Phase A, once: the plan is made over the same Funcs the first pass rewrites, and a retry
        // rebuilds them because the rewrite consumed them.
        let default_heap = HeapPlan::default();
        let mut pristine_funcs = {
            let _scope = crate::profile::scope("lane.A.build_funcs");
            let ctx = self.indexes.default_analysis_ctx(&default_heap);
            build_funcs(self.table, &self.original_names, &ctx)
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
                        if let Some(old_sentinel) = &enum_plan.sentinel_none {
                            self.defined.remove(old_sentinel);
                            self.live.remove(old_sentinel);
                        }
                        *self.table = raw_table.clone();
                        enum_plan = crate::plan::enum_repr::specialize_table(
                            self.table,
                            self.reachable,
                            &self.original_names,
                            &enum_excluded,
                            self.opts,
                        );
                        self.install_enum_sentinels(&enum_plan);
                        let ctx = self.indexes.default_analysis_ctx(&default_heap);
                        pristine_funcs = build_funcs(self.table, &self.original_names, &ctx);
                    }
                }
            }
        }
        let empty_native_bodies = BTreeMap::new();
        let (byte_views, heap) = refresh_heap_analyses(
            self.table,
            &pristine_funcs,
            &empty_native_bodies,
            self.opts,
            &enum_plan,
        );
        self.indexes.byte_views = byte_views;
        self.indexes.heap = heap;
        // Effects-summary coverage (phase 5, W5b): analysis only, stderr only, computed on demand.
        if std::env::var("RCGJS_SUMMARY_REPORT").is_ok_and(|value| value == "1") {
            crate::analysis::effects::print_report(self.table, &pristine_funcs);
        }
        let pristine_table = self.table.clone();
        StageA {
            unspecialized_table,
            enum_excluded,
            enum_plan,
            pristine_funcs,
            pristine_table,
        }
    }

    /// Stage 2: run pre-repr Phase O, Phase B1 native container planning, drop-glue pruning,
    /// post-repr Phase O, and universal `NativeString` format-kernel commitment.
    fn run_pre_repr_o_and_b1(&mut self, stage_a: &StageA) -> StageB1 {
        let empty_native_bodies = BTreeMap::new();
        let mut planned_funcs = if self.opts.optimizer == OptimizerMode::Report
            && self.opts.analyzes(OptimizerPass::AstDce)
        {
            // Under `js-opt=report`, `pristine_funcs` keeps `fold_runtime_checks = false` so the
            // emitted program stays byte-identical to `js-opt=off`, while `planned_funcs` folds
            // runtime checks just as `js-opt=on` does so O-group and B1 repr accounting match `on`.
            let mut plan_opts = self.opts.clone();
            plan_opts.optimizer = OptimizerMode::On;
            let plan_ctx = Ctx {
                opts: &plan_opts,
                ..self.indexes.ctx()
            };
            build_funcs(self.table, &self.original_names, &plan_ctx)
        } else {
            stage_a.pristine_funcs.clone()
        };
        let mut o_stats =
            crate::profile::time("lane.O", || self.run_o_if_enabled(&mut planned_funcs));
        let (byte_views, heap) = refresh_heap_analyses(
            self.table,
            &planned_funcs,
            &empty_native_bodies,
            self.opts,
            &stage_a.enum_plan,
        );
        self.indexes.byte_views = byte_views;
        self.indexes.heap = heap;
        let pre_repr_native_string = self.indexes.heap.native_string_enabled;
        let pre_repr_native_vec = self.indexes.heap.native_vec_enabled;

        // Phase B1: whole-type native container lowering (`Box<T, Global>` drop-glue elision and
        // order-unobserved `BTreeMap<K, V, Global>` -> JS `Map`).
        let repr_ran = self.opts.analyzes(OptimizerPass::NativeRepresentations);
        let planned_repr = if repr_ran {
            let repr_live = crate::analysis::heap::filter_live_for_repr(
                self.table,
                &planned_funcs,
                &self.live,
                self.opts,
                &self.indexes.heap,
            );
            let repr_cx = crate::plan::native_types::ReprCx {
                opts: self.opts,
                drop_glue: &self.indexes.drop_glue,
                heap: &self.indexes.heap,
            };
            crate::profile::time("lane.B1.native_types", || {
                crate::plan::native_types::plan_and_commit(
                    self.table,
                    &planned_funcs,
                    &repr_live,
                    &repr_cx,
                )
            })
        } else {
            ReprOutcome::default()
        };
        let planned_body_names: Vec<JsName> = planned_repr.bodies.keys().cloned().collect();
        let planned_has_native_map = planned_repr.has_native_map;
        let mut planning_table = self.table.clone();
        let emitting_plans = self.opts.optimizer == OptimizerMode::On;
        if self.opts.analyzes(OptimizerPass::AstDce) {
            crate::opt::prune_drops::run(&planning_table, &mut planned_funcs, &self.indexes.heap);
            let mut excluded: BTreeMap<JsName, Result<Func, Decline>> = BTreeMap::new();
            for name in planned_repr.bodies.keys() {
                if let Some(entry) = planned_funcs.remove(name) {
                    excluded.insert(name.clone(), entry);
                }
            }
            let ctx = self.indexes.ctx();
            let post_o = o_group_with_mode(
                &mut planning_table,
                &mut planned_funcs,
                &ctx,
                &self.live,
                true,
            );
            crate::opt::prune_drops::run(&planning_table, &mut planned_funcs, &self.indexes.heap);
            planned_funcs.extend(excluded);
            merge_o_stats(&mut o_stats, post_o);
        }
        let mut funcs = if emitting_plans {
            planned_funcs.clone()
        } else {
            stage_a.pristine_funcs.clone()
        };
        let mut repr = if emitting_plans {
            planned_repr
        } else {
            planned_repr.unapplied()
        };
        let work_table = if emitting_plans {
            planning_table.clone()
        } else {
            self.table.clone()
        };
        let (byte_views, mut heap) = refresh_heap_analyses(
            &work_table,
            &funcs,
            &repr.bodies,
            self.opts,
            &stage_a.enum_plan,
        );
        if !repr.bodies.is_empty() {
            // `repr.bodies` were emitted against `pre_repr_native_string` / `pre_repr_native_vec`;
            // keep the post-repr heap plan in lockstep so native bodies and lowered items agree.
            heap.native_string_enabled &= pre_repr_native_string;
            heap.native_vec_enabled &= pre_repr_native_vec;
        }
        self.indexes.byte_views = byte_views;
        self.indexes.heap = heap;

        // Universal `NativeString` call-site format kernel (`format!`/`write!`/`writeln!`): runs
        // once `heap.native_string_enabled` is finalized after post-repr inlining and before
        // `abi_flat` and Phase C, so helper functions containing `format!`/`write!` remain
        // baseline during inlining and every proven site lowers inline to `{ s, cap }` / `out.s +=`.
        if emitting_plans && self.indexes.heap.native_string_enabled {
            let skip_bodies: BTreeSet<JsName> = repr.bodies.keys().cloned().collect();
            let fmt_touched =
                crate::lower::semantic_ops::fmt_kernel::commit_universal_native_string_sites(
                    &work_table,
                    &mut funcs,
                    &self.live,
                    &skip_bodies,
                    &self.indexes.items,
                );
            for name in &fmt_touched {
                if let Some(func) = funcs.get(name) {
                    planned_funcs.insert(name.clone(), func.clone());
                }
            }
            repr.touched.extend(fmt_touched);
        }

        StageB1 {
            planned_funcs,
            planning_table,
            funcs,
            work_table,
            o_stats,
            repr_ran,
            repr,
            planned_body_names,
            planned_has_native_map,
            pre_repr_native_string,
            pre_repr_native_vec,
        }
    }

    /// Stage 3: plan and cost-price the Phase B2 flat direct-call ABI, install ABI clones into
    /// `work_table` / `funcs`, and empty the plan under `js-opt=report`.
    fn plan_and_price_abi(&mut self, stage_a: &StageA, stage_b1: &mut StageB1) -> StageB2 {
        // B2 plans against the same post-O, post-representation map D9 measured. Report mode
        // records that plan and empties it before emission.
        let abi_funcs = stage_b1.planned_funcs.clone();
        let abi_excluded: BTreeSet<crate::jir::semantic::ItemId> = stage_b1
            .planned_body_names
            .iter()
            .filter_map(|source| {
                stage_a
                    .pristine_funcs
                    .get(source)?
                    .as_ref()
                    .ok()
                    .map(|func| func.item)
            })
            .collect();
        let mut plan = crate::profile::time("lane.B2.abi_flat", || {
            crate::plan::abi_flat::plan_excluding(
                &stage_b1.planning_table,
                &abi_funcs,
                self.opts,
                &abi_excluded,
            )
        });
        let abi_applied = self.opts.applies(OptimizerPass::Sroa);
        if !plan.is_empty() {
            let _scope = crate::profile::scope("lane.B2.physical");
            let mut reuse = crate::plan::lowering_reuse::Reuse::physical();
            let ctx = self.indexes.ctx();
            let (selected, _) = crate::plan::physical_cost::evaluate_groups(
                &stage_b1.planning_table,
                &stage_b1.planned_funcs,
                &plan,
                self.opts,
                abi_applied,
                |candidate| {
                    physical_program_metrics(
                        &stage_b1.planning_table,
                        &stage_b1.planned_funcs,
                        candidate,
                        &ctx,
                        stage_b1.planned_has_native_map,
                        &mut reuse,
                    )
                },
            );
            plan = selected;
        }
        let abi_clone_names: BTreeSet<JsName> = plan.clones().keys().cloned().collect();
        if self.opts.optimizer == OptimizerMode::On {
            plan.install_clones(&mut stage_b1.work_table, &mut stage_b1.funcs);
            for clone in &abi_clone_names {
                self.names.push(clone.clone());
                self.defined.insert(clone.clone());
                self.live.insert(clone.clone());
            }
            self.names.sort();
            self.names.dedup();
        }
        // `js-opt=report` plans so it can report the plan, and prints the program `js-opt=off`
        // prints: the would-be plan is recorded here and then emptied, before phase B, so nothing
        // is rewritten, no spill register is added and no item is touched. Under `js-opt=on` the
        // plan is reported after the loop instead, since a retry is what the sidecar has to
        // describe.
        let mut reported = None;
        if !abi_applied {
            reported = plan.json(false);
            plan = plan.emptied();
        }
        StageB2 {
            plan,
            abi_applied,
            abi_clone_names,
            reported,
        }
    }

    /// Stage 4: register planned spill/sentinel/helper names, rewrite byte-string const items, and
    /// allocate ES2018 BigInt hoist names before Phase C.
    fn prepare_synthetic_names(
        &mut self,
        plan: &AbiPlan,
        work_table: &BTreeMap<JsName, LinkItem>,
    ) -> SyntheticNames {
        // A spill register is an item the lane is about to add, so a lowering that names one names
        // something the link step has prepared. Known before phase C prints anything, because that
        // is where a lowering is checked against these two sets.
        for register in plan.spill_names() {
            self.defined.insert(register.clone());
            self.live.insert(register.clone());
        }
        if let Some(sentinel) = plan.sentinel_err_name() {
            self.defined.insert(sentinel.clone());
            self.live.insert(sentinel.clone());
        }
        for helper in crate::lower::js::i64_pair::HELPER_NAMES {
            if !self.table.contains_key(*helper) {
                let helper_name = JsName::new((*helper).to_owned());
                self.defined.insert(helper_name.clone());
                self.live.insert(helper_name);
            }
        }
        let mut bstr_const_items = Vec::new();
        if crate::lower::bstr::enabled() {
            for (name, item) in self.table.iter_mut() {
                if item.kind == rcgjs_object::object::ItemKind::Const
                    && name.as_str().starts_with("loc$h")
                    && self.live.contains(name)
                {
                    if let Some(rewritten) =
                        crate::link::hoist_consts::rewrite_bstr_location_const_item(&item.js)
                    {
                        item.js = rewritten;
                    }
                }
            }
            for (name, item) in self.table.iter() {
                if item.kind == rcgjs_object::object::ItemKind::Const
                    && name.as_str().starts_with("s$h")
                    && self.live.contains(name)
                {
                    if let Some(bs_item) = crate::link::hoist_consts::synthesize_bstr_const_item(
                        name.as_str(),
                        &item.js,
                    ) {
                        if !self.table.contains_key(&bs_item.name)
                            && !self.defined.contains(&bs_item.name)
                        {
                            self.defined.insert(bs_item.name.clone());
                            self.live.insert(bs_item.name.clone());
                            bstr_const_items.push(bs_item);
                        }
                    }
                }
            }
        }
        // ES2018 spells a BigInt constant as a call that parses its digits whenever it runs, so
        // phase C names a module-level binding for each one instead (`lower/bigint.rs`). The prefix
        // is allocated around every name the program spells, its first-pass clones and registers
        // included; a retry spells a subset of those.
        let bigint_names = if self.opts.ecma_target == crate::opts::EcmaTarget::Es2018 {
            crate::lower::bigint::BigIntNames::allocate(work_table, plan.spill_names())
        } else {
            None
        };
        SyntheticNames {
            bstr_const_items,
            bigint_names,
        }
    }

    /// Stage 5: rewrite with Phase B2 + SoA planning and run Phase C transactional lowering,
    /// retrying from pristine inputs whenever a planned or enum-specialized item declines.
    fn lower_with_retry(
        &mut self,
        stage_a: &mut StageA,
        stage_b1: &mut StageB1,
        stage_b2: &mut StageB2,
        synth: &SyntheticNames,
    ) -> LoweringResult {
        let emitting_plans = self.opts.optimizer == OptimizerMode::On;
        let mut funcs = std::mem::take(&mut stage_b1.funcs);
        let mut plan_retries = 0u32;
        let mut enum_retries = 0u32;
        let mut soa_allowed = true;
        let mut active_soa_helpers: Vec<JsName> = Vec::new();

        loop {
            {
                let _rewrite_scope = crate::profile::scope("lane.rewrite");
                stage_b2.plan.rewrite(&mut funcs, &stage_b1.work_table);
            }
            for helper in active_soa_helpers.drain(..) {
                self.defined.remove(&helper);
                self.live.remove(&helper);
            }
            let soa_plan = if soa_allowed && emitting_plans {
                Arc::new(crate::plan::soa_vec::plan(
                    &stage_b1.work_table,
                    &funcs,
                    &stage_b1.repr.bodies,
                    &self.live,
                    self.opts,
                    &self.indexes.heap,
                ))
            } else {
                Arc::new(SoaPlan::default())
            };
            active_soa_helpers = soa_plan.all_helper_names();
            for helper in &active_soa_helpers {
                self.defined.insert(helper.clone());
                self.live.insert(helper.clone());
            }

            // Phase C.
            let report_probe = (self.opts.optimizer == OptimizerMode::Report)
                .then_some((&stage_b1.planning_table, &stage_b1.planned_funcs));
            let (lowered, retry) = crate::profile::time("lane.C.lower_all", || {
                let ctx = self.indexes.ctx();
                let lcx = LowerAllCx {
                    table: &stage_b1.work_table,
                    names: &self.names,
                    ctx: &ctx,
                    report_probe,
                    plan: &stage_b2.plan,
                    repr: &stage_b1.repr,
                    soa: &soa_plan,
                    defined: &self.defined,
                    live: &self.live,
                    fault: self.fault,
                    bigints: synth.bigint_names.as_ref(),
                };
                lower_all(&lcx, funcs)
            });

            let enum_conflict_items = find_enum_conflicts(stage_a, &lowered, enum_retries);
            let needs_retry = !enum_conflict_items.is_empty()
                || (retry
                    && (!stage_b2.plan.is_empty()
                        || !stage_b1.repr.touched.is_empty()
                        || !soa_plan.is_empty()));
            if needs_retry {
                if !soa_plan.is_empty() {
                    soa_allowed = false;
                }
                for helper in active_soa_helpers.drain(..) {
                    self.defined.remove(&helper);
                    self.live.remove(&helper);
                }
                plan_retries += 1;
                if !enum_conflict_items.is_empty() {
                    enum_retries += 1;
                }
                funcs = self.reset_for_retry(stage_a, stage_b1, stage_b2, &enum_conflict_items);
                continue;
            }
            return LoweringResult {
                lowered,
                final_soa_plan: soa_plan,
                plan_retries,
            };
        }
    }

    fn reset_for_retry(
        &mut self,
        stage_a: &mut StageA,
        stage_b1: &mut StageB1,
        stage_b2: &mut StageB2,
        enum_conflict_items: &[JsName],
    ) -> BTreeMap<JsName, Result<Func, Decline>> {
        let _scope = crate::profile::scope("lane.retry");
        // BOTH plans are emptied (D1): the retried run rebuilds pristine Funcs and rewrites
        // nothing, so nothing it touched can decline. Each keeps its refusal accounting, so the
        // sidecar still says what was considered on a run that kept none of it.
        stage_b2.plan = stage_b2.plan.emptied();
        stage_b1.repr = stage_b1.repr.emptied();
        for clone in &stage_b2.abi_clone_names {
            self.defined.remove(clone);
            self.live.remove(clone);
        }
        self.names.clone_from(&self.original_names);
        if !enum_conflict_items.is_empty() {
            if let Some(raw_table) = &stage_a.unspecialized_table {
                if let Some(old_sentinel) = &stage_a.enum_plan.sentinel_none {
                    self.defined.remove(old_sentinel);
                    self.live.remove(old_sentinel);
                }
                crate::plan::enum_repr::collect_item_signature_types(
                    raw_table,
                    enum_conflict_items,
                    &mut stage_a.enum_excluded,
                );
                *self.table = raw_table.clone();
                stage_a.enum_plan = crate::plan::enum_repr::specialize_table(
                    self.table,
                    self.reachable,
                    &self.original_names,
                    &stage_a.enum_excluded,
                    self.opts,
                );
                self.install_enum_sentinels(&stage_a.enum_plan);
            } else {
                *self.table = stage_a.pristine_table.clone();
            }
        } else {
            *self.table = stage_a.pristine_table.clone();
        }
        let mut funcs = {
            let ctx = self.indexes.ctx();
            build_funcs(self.table, &self.original_names, &ctx)
        };
        // The O group re-runs on the pristine table and Funcs (both `run_o` and `post_o`,
        // matching a plan-less run), so the retried program is byte-identical to
        // `js-jir-flat-abi=off`.
        stage_b1.o_stats = self.run_o_if_enabled(&mut funcs);
        stage_b1.work_table = self.table.clone();
        let emitting_plans = self.opts.optimizer == OptimizerMode::On;
        if emitting_plans && self.opts.analyzes(OptimizerPass::AstDce) {
            let ctx = self.indexes.ctx();
            let post_o =
                o_group_with_mode(&mut stage_b1.work_table, &mut funcs, &ctx, &self.live, true);
            merge_o_stats(&mut stage_b1.o_stats, post_o);
        }
        let (byte_views, mut heap) = refresh_heap_analyses(
            &stage_b1.work_table,
            &funcs,
            &stage_b1.repr.bodies,
            self.opts,
            &stage_a.enum_plan,
        );
        if !stage_b1.repr.bodies.is_empty() {
            heap.native_string_enabled &= stage_b1.pre_repr_native_string;
            heap.native_vec_enabled &= stage_b1.pre_repr_native_vec;
        }
        self.indexes.byte_views = byte_views;
        self.indexes.heap = heap;
        funcs
    }

    /// Stage 6: hoist constant tables and ES2018 BigInts and insert synthesized spill, BigInt, and
    /// byte-string constant items into `self.table`.
    fn install_synthesized_items(
        &mut self,
        lowered: &mut [Lowered],
        plan: &AbiPlan,
        enum_plan: &EnumReprPlan,
        abi_clone_names: &BTreeSet<JsName>,
        synth: SyntheticNames,
    ) -> bool {
        // The spill registers the plan needs are items like any other, and they have to be in the
        // table and reachable before a lowering that names them is checked against either.
        let spills = plan.spill_items();
        let mut added_items =
            !spills.is_empty() || enum_plan.sentinel_none.is_some() || !abi_clone_names.is_empty();
        if self.opts.optimizer == OptimizerMode::On {
            added_items |= crate::link::hoist_consts::run(
                self.table,
                lowered,
                &mut self.defined,
                &mut self.live,
            );
            if let Some(names) = &synth.bigint_names {
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
            match self.table.entry(spill.name.clone()) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(spill);
                }
                std::collections::btree_map::Entry::Occupied(slot) => self.dcx.err(format!(
                    "rustc_codegen_js: the flat direct-call ABI allocated the spill register name                  `{}`, which the program already defines; this is a compiler bug, please report it",
                    slot.key().as_str()
                )),
            }
        }

        // One `const` per BigInt constant the lowerings name, plus the `BigInt` test and the
        // throwing function the guarded uses name, the same way: prepared before `apply` checks a
        // lowering against `defined` and `live`, and never over an item the program defines (the
        // prefix was allocated around all of them). A binding no installed item ends up naming is
        // unreachable and is not printed.
        let constants: BTreeSet<&String> = lowered
            .iter()
            .filter_map(|(_, result, _, _)| result.as_ref().ok())
            .flat_map(|lowering| &lowering.bigints)
            .collect();
        let mut added_bigints = false;
        let support = match &synth.bigint_names {
            Some(names) if !constants.is_empty() => names.support().to_vec(),
            _ => Vec::new(),
        };
        let hoisted = constants.into_iter().filter_map(|digits| {
            synth
                .bigint_names
                .as_ref()
                .and_then(|names| names.item(digits))
        });
        for constant in support.into_iter().chain(hoisted) {
            self.defined.insert(constant.name.clone());
            self.live.insert(constant.name.clone());
            match self.table.entry(constant.name.clone()) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(constant);
                    added_bigints = true;
                }
                std::collections::btree_map::Entry::Occupied(slot) => self.dcx.err(format!(
                    "rustc_codegen_js: the BigInt constant hoist allocated the name `{}`, which the \
                     program already defines; this is a compiler bug, please report it",
                    slot.key().as_str()
                )),
            }
        }
        let mut added_bstr_consts = false;
        for bs_item in synth.bstr_const_items {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                self.table.entry(bs_item.name.clone())
            {
                slot.insert(bs_item);
                added_bstr_consts = true;
            }
        }
        added_items || added_bigints || added_bstr_consts
    }

    /// Stage 7: apply every lowered declaration into `self.table`, emit diagnostics for any
    /// declined item, install required runtime helper items, and assemble the final [`LaneRun`].
    fn commit_outcomes(
        self,
        result: LoweringResult,
        stage_b1: &StageB1,
        stage_b2: &StageB2,
        added_items: bool,
    ) -> LaneRun {
        let mut passes = Passes::default();
        let mut range_asserts: u64 = 0;
        let mut range_comparisons: u64 = 0;
        let mut witnesses_recorded: u64 = u64::from(
            stage_b1.o_stats.witnesses_recorded
                + stage_b1.o_stats.inline.witnesses_recorded
                + stage_b1.o_stats.devirt.witnesses_recorded,
        );
        let mut witnesses_failed: u64 = u64::from(
            stage_b1.o_stats.witnesses_failed
                + stage_b1.o_stats.inline.witnesses_failed
                + stage_b1.o_stats.devirt.witnesses_failed,
        );
        let mut outcomes = BTreeMap::new();
        for (name, lowering_res, stats, alloc) in result.lowered {
            range_asserts += u64::from(stats.asserts_removed);
            range_comparisons += u64::from(stats.range_comparisons_folded);
            witnesses_recorded += u64::from(stats.witnesses_recorded);
            witnesses_failed += u64::from(stats.witnesses_failed);
            let js_alloc_json = lowering_res.as_ref().ok().map(|lowered| {
                crate::analysis::js_alloc::JsAllocSites::of_with_origins(
                    &lowered.decl,
                    &lowered.js_alloc_origins,
                )
                .to_json()
            });
            if !self.table.contains_key(&name)
                && lowering_res.is_ok()
                && stage_b2.abi_clone_names.contains(&name)
            {
                if let Some(clone) = stage_b1.work_table.get(&name).cloned() {
                    self.table.insert(name.clone(), clone);
                }
            }
            let Some(item) = self.table.get_mut(&name) else {
                continue;
            };
            let class = item
                .jir
                .summary
                .as_ref()
                .map(|s| s.class)
                .unwrap_or_default();
            let bytes_baseline = item.js.len();
            let mut host_args = stats.host_args.clone();
            let mut slice_iterator_final_emitted = 0;
            let disposition = match lowering_res {
                Err(decline) => LaneDisposition::Fallback {
                    decline,
                    refs_equal: true,
                },
                Ok(lowered) => match apply(self.dcx, item, lowered, &self.defined, &self.live) {
                    Ok(applied) => {
                        if self.opts.applies(OptimizerPass::AstDce) {
                            host_args.final_emitted = stats.host_args.would_lower;
                            slice_iterator_final_emitted = stats.slice_iterator_loops_would_lower;
                        }
                        record_applied_passes(
                            &mut passes,
                            item,
                            &name,
                            &stats,
                            &stage_b1.o_stats,
                            self.opts,
                        );
                        LaneDisposition::Jir {
                            bytes_jir: applied.bytes,
                            bytes_min: applied.bytes_min,
                            refs_equal: !applied.refs_changed,
                        }
                    }
                    Err(decline) => LaneDisposition::Fallback {
                        decline,
                        refs_equal: false,
                    },
                },
            };
            let outcome = LaneOutcome {
                name: name.clone(),
                class,
                disposition,
                bytes_baseline,
                alloc_json: alloc,
                js_alloc_json,
                host_args,
                slice_iterator_would_lower: stats.slice_iterator_loops_would_lower,
                slice_iterator_final_emitted,
            };
            outcomes.insert(name, outcome);
        }

        // JIR is the only lowering lane: any offered item that fails to lower is a hard link error.
        for outcome in outcomes.values().filter(|o| !o.is_jir()) {
            let mut diag = self.dcx.struct_err(format!(
                "rustc_codegen_js: JIR lowering of `{}` failed ({:?})",
                outcome.name.as_str(),
                outcome.decline()
            ));
            if let Some(item) = self.table.get(&outcome.name) {
                diag.note(format!("in `{}`", item.debug_path));
            }
            diag.emit();
        }

        let referenced: BTreeSet<JsName> = self
            .table
            .values()
            .flat_map(|item| item.refs.iter().cloned())
            .collect();
        let existing: BTreeSet<JsName> = self.table.keys().cloned().collect();
        let helper_items = crate::lower::js::i64_pair::needed_helper_items(&referenced, &existing);
        let soa_helper_items = crate::lower::js::soa::needed_helper_items(
            &result.final_soa_plan,
            &referenced,
            &existing,
        );
        let added_items = added_items || !helper_items.is_empty() || !soa_helper_items.is_empty();
        for helper in helper_items.into_iter().chain(soa_helper_items) {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                self.table.entry(helper.name.clone())
            {
                slot.insert(helper);
            }
        }

        LaneRun {
            outcomes,
            passes,
            o: stage_b1.o_stats.clone(),
            range_asserts,
            range_comparisons,
            witnesses_recorded,
            witnesses_failed,
            abi_json: if stage_b2.abi_applied {
                stage_b2.plan.json(true)
            } else {
                stage_b2.reported.clone()
            },
            // The representation plan's accounting (Phase 4, D12): present whenever its analysis
            // ran, filled by the solver in report and on alike.
            repr: stage_b1.repr_ran.then(|| stage_b1.repr.block.clone()),
            plan_retries: result.plan_retries,
            added_items,
            byte_views: self.indexes.byte_views,
            native_vec_enabled: self.indexes.heap.native_vec_enabled,
        }
    }
}

/// Records pass counts and item optimization metadata for a successfully applied item.
fn record_applied_passes(
    passes: &mut Passes,
    item: &mut LinkItem,
    name: &JsName,
    stats: &OptStats,
    o_stats: &OGroup,
    opts: &Opts,
) {
    // What `js-opt-report`'s `passes.applied` reads: a pass is applied to an item only when it
    // changed the item AND that item's lowering is what was printed. Under `js-opt=report` the
    // change was made to a clone and dropped, so it counts as `would_apply` and nothing else.
    if stats.sroa_changed {
        passes.would_apply.sroa += 1;
        if opts.applies(OptimizerPass::Sroa) {
            passes.applied.sroa += 1;
            item.optimization.applied.insert("sroa".to_owned());
        }
    }
    if stats.ast_dce_changed || o_stats.dce_items.contains(name) {
        passes.would_apply.ast_dce += 1;
        if opts.applies(OptimizerPass::AstDce) {
            passes.applied.ast_dce += 1;
            item.optimization.applied.insert("ast-dce".to_owned());
        }
    }
    // The O group's item counts. Only the lane sidecar carries them: the report's `passes.applied`
    // names are v1's five, and `inline`/`devirt` are not among them, so `item.optimization` is left
    // alone.
    if o_stats.inline.items.contains(name) {
        passes.would_apply.inline += 1;
        if opts.applies(OptimizerPass::AstDce) {
            passes.applied.inline += 1;
        }
    }
    if o_stats.devirt.items.contains(name) {
        passes.would_apply.devirt += 1;
        if opts.applies(OptimizerPass::AstDce) {
            passes.applied.devirt += 1;
        }
    }
}

fn find_enum_conflicts(stage_a: &StageA, lowered: &[Lowered], enum_retries: u32) -> Vec<JsName> {
    if stage_a.enum_plan.is_empty() || enum_retries >= 2 {
        return Vec::new();
    }
    let declined_c: Vec<JsName> = lowered
        .iter()
        .filter_map(|(n, res, _, _)| res.is_err().then_some(n.clone()))
        .collect();
    if declined_c.is_empty() {
        return Vec::new();
    }
    let Some(raw_table) = &stage_a.unspecialized_table else {
        return Vec::new();
    };
    let mut sig_tys = BTreeSet::new();
    crate::plan::enum_repr::collect_item_signature_types(raw_table, &declined_c, &mut sig_tys);
    if sig_tys.iter().any(|t| {
        stage_a.enum_plan.decisions.contains_key(t)
            || stage_a
                .enum_plan
                .smi_u32_fields
                .iter()
                .any(|(st, _)| st == t)
    }) {
        declined_c
    } else {
        Vec::new()
    }
}

/// Recomputes the reachable byte-view policy, heap plan, and post-SSA SMI `u32` fields together
/// so the `raw_byte_derefs` guard cannot drift between initial lowering and retries.
fn refresh_heap_analyses(
    table: &BTreeMap<JsName, LinkItem>,
    funcs: &BTreeMap<JsName, Result<Func, Decline>>,
    native_bodies: &BTreeMap<JsName, NativeBody>,
    opts: &Opts,
    enum_plan: &EnumReprPlan,
) -> (ByteViewPlan, HeapPlan) {
    let byte_views =
        crate::analysis::byte_views::analyze_reachable(table, funcs, native_bodies, &opts.roots);
    let mut heap =
        crate::analysis::heap::analyze_reachable(table, funcs, native_bodies, opts, byte_views);
    heap.smi_u32_fields = if byte_views.raw_byte_derefs {
        BTreeSet::new()
    } else {
        crate::analysis::heap::filter_smi_u32_fields_post_ssa(
            table,
            funcs,
            &enum_plan.smi_u32_fields,
        )
    };
    (byte_views, heap)
}

/// What every lowering of one lane pass shares: the options and the program-wide indexes/analyses.
pub(super) struct Ctx<'a> {
    pub(super) opts: &'a Opts,
    pub(super) items: &'a ItemIndex,
    pub(super) drop_glue: &'a DropGlueIndex,
    pub(super) vtables: &'a VtableIndex,
    pub(super) byte_views: ByteViewPlan,
    pub(super) heap: &'a HeapPlan,
}

/// The names the lane offers to the lowerer, sorted and deduplicated.
///
/// `reachable` is a walk order; the sidecar and the binary search over its rows are by name. An
/// item is offered when it carries a decoded body, its class is promoted, it is not on the allow
/// list, and `js-lane-only` (when given) names it — see the file header.
fn offered(table: &BTreeMap<JsName, LinkItem>, reachable: &[JsName], opts: &Opts) -> Vec<JsName> {
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
pub(super) fn lower_input<'a>(
    name: &'a JsName,
    item: &'a LinkItem,
    ctx: &Ctx<'a>,
    spills: &'a [JsName],
) -> Option<LowerInput<'a>> {
    lower_input_with_retargets(name, item, ctx, spills, crate::lower::EMPTY_CALL_RETARGETS)
}

fn lower_input_with_retargets<'a>(
    name: &'a JsName,
    item: &'a LinkItem,
    ctx: &Ctx<'a>,
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
        final_lowering: false,
        call_retargets,
        bigints: None,
        byte_views: ctx.byte_views,
        heap: Some(ctx.heap),
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
    has_native_map: bool,
    reuse: &mut crate::plan::lowering_reuse::Reuse,
) -> Option<crate::plan::physical_cost::ProgramMetrics> {
    crate::profile::bump(crate::profile::Counter::TrialPhysical);
    reuse.begin_trial(source_table, source_funcs, ctx.opts);
    let mut opts = ctx.opts.clone();
    opts.optimizer = OptimizerMode::On;
    opts.minify = crate::opts::MinifyMode::All;
    let measured_ctx = Ctx {
        opts: &opts,
        items: ctx.items,
        drop_glue: ctx.drop_glue,
        vtables: ctx.vtables,
        byte_views: ctx.byte_views,
        heap: ctx.heap,
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
        input.nested_clone.map = has_native_map;
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

    let mut reachable = crate::plan::native_types::measurement_reach(&table, &opts.roots);
    if opts.analyzes(OptimizerPass::LinkDedup)
        && crate::link_dedup::merge_duplicates_for_measurement(
            &mut table,
            &reachable,
            opts.source_map,
        )
    {
        reachable = crate::plan::native_types::measurement_reach(&table, &opts.roots);
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
pub(super) fn propagated_alloc_term(
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
pub(super) fn build_funcs(
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

type ReportProbe<'a> = (
    &'a BTreeMap<JsName, LinkItem>,
    &'a BTreeMap<JsName, Result<Func, Decline>>,
);

fn opt_debug_func() -> Option<&'static str> {
    static FILTER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FILTER
        .get_or_init(|| std::env::var("RCGJS_OPT_DEBUG_FUNC").ok())
        .as_deref()
}

fn repr_debug_solve() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("RCGJS_REPR_DEBUG_SOLVE").is_ok())
}

/// Shared immutable inputs for [`prepare_opt_func`].
struct OptFuncCx<'a> {
    opts: &'a Opts,
    report_probe: Option<ReportProbe<'a>>,
    bounds_facts: Option<&'a ProgramBoundsFacts>,
}

/// Shared immutable inputs for Phase C [`lower_all`].
struct LowerAllCx<'a> {
    table: &'a BTreeMap<JsName, LinkItem>,
    names: &'a [JsName],
    ctx: &'a Ctx<'a>,
    report_probe: Option<ReportProbe<'a>>,
    plan: &'a AbiPlan,
    repr: &'a ReprOutcome,
    soa: &'a SoaPlan,
    defined: &'a BTreeSet<JsName>,
    live: &'a BTreeSet<JsName>,
    fault: Option<&'a str>,
    bigints: Option<&'a crate::lower::bigint::BigIntNames>,
}

/// Runs the per-function optimizer and alloc-site counter for one item in Phase C.
fn prepare_opt_func(
    name: &JsName,
    func: &mut Func,
    interns: &crate::jir::Interns,
    opt_cx: &OptFuncCx<'_>,
) -> Result<(OptStats, Option<String>), Decline> {
    let mut stats = OptStats::default();
    let saved =
        (opt_cx.report_probe.is_none() && crate::opt::witness::enabled()).then(|| func.clone());
    let probe = if let Some((probe_table, probe_funcs)) = opt_cx.report_probe {
        if let (Some(probe_item), Some(Ok(probe_func))) =
            (probe_table.get(name), probe_funcs.get(name))
        {
            if let Some(probe_interns) = probe_item.jir.interns() {
                let mut probe = probe_func.clone();
                if crate::opt::optimize_with_facts(
                    &mut probe,
                    &probe_interns.types,
                    probe_interns,
                    opt_cx.opts,
                    opt_cx.bounds_facts,
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
        crate::opt::optimize_with_facts(
            func,
            &interns.types,
            interns,
            opt_cx.opts,
            opt_cx.bounds_facts,
            &mut stats,
        )
    };
    match probe {
        Err(Decline::Proof(_)) if saved.is_some() => {
            *func = saved.expect("checked above");
            let recorded = stats.witnesses_recorded;
            let failed = stats.witnesses_failed.max(1);
            stats = OptStats {
                witnesses_recorded: recorded,
                witnesses_failed: failed,
                ..OptStats::default()
            };
            let alloc = crate::analysis::alloc_sites::AllocSites::of(func, interns).to_json();
            Ok((stats, Some(alloc)))
        }
        Err(decline) => {
            if opt_debug_func().is_some_and(|filter| name.as_str().contains(filter)) {
                eprintln!("opt-func: {} declined: {decline}", name.as_str());
            }
            Err(decline)
        }
        Ok(()) => {
            // The `RCGJS_O_DEBUG_FUNC` precedent, after the per-item passes: the exact `Func`
            // the alloc analysis counts and the printer prints.
            if opt_debug_func().is_some_and(|filter| name.as_str().contains(filter)) {
                eprintln!("opt-func: {} = {func:#?}", name.as_str());
            }
            // Over the final `Func`, so what is counted is what is printed. Counted in every
            // optimizer mode: `js-opt=report` rewrites nothing and this is what it has to report.
            let alloc = crate::analysis::alloc_sites::AllocSites::of(func, interns).to_json();
            Ok((stats, Some(alloc)))
        }
    }
}

/// Pre-optimizes all eligible functions and computes whole-program ASCII facts when `js-str=bytes`
/// is enabled.
fn prepare_bstr_ascii(
    lcx: &LowerAllCx<'_>,
    funcs: &mut BTreeMap<JsName, Result<Func, Decline>>,
    opt_cx: &OptFuncCx<'_>,
    prepared_meta: &mut BTreeMap<JsName, (OptStats, Option<String>)>,
) -> crate::analysis::ascii::AsciiFacts {
    for name in lcx.names {
        if lcx.repr.bodies.contains_key(name) {
            continue;
        }
        let Some(item) = lcx.table.get(name) else {
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
        match prepare_opt_func(name, func, interns, opt_cx) {
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
            if lcx.repr.bodies.contains_key(name) {
                return None;
            }
            res.as_ref().ok().map(|f| (name.clone(), f))
        })
        .collect();
    let roots: BTreeSet<JsName> = lcx
        .ctx
        .opts
        .roots
        .iter()
        .map(|r| JsName::new(r.clone()))
        .collect();
    crate::analysis::ascii::compute(
        &live_funcs,
        lcx.table,
        lcx.repr,
        lcx.plan,
        &roots,
        lcx.defined,
        lcx.live,
    )
}

/// Phase C: optimize and print each item. Returns the declarations and whether the run must retry.
///
/// Nothing here touches the table: a retry has to be decided before any item is replaced.
fn lower_all(
    lcx: &LowerAllCx<'_>,
    mut funcs: BTreeMap<JsName, Result<Func, Decline>>,
) -> (Vec<Lowered>, bool) {
    let bounds_facts = (lcx.ctx.opts.optimizer == OptimizerMode::On).then(|| {
        crate::analysis::arena::analyze(
            lcx.table,
            &funcs,
            &lcx.repr.bodies,
            lcx.live,
            lcx.ctx.opts,
            lcx.ctx.heap,
        )
    });
    let opt_cx = OptFuncCx {
        opts: lcx.ctx.opts,
        report_probe: lcx.report_probe,
        bounds_facts: bounds_facts.as_ref(),
    };
    let mut prepared_meta: BTreeMap<JsName, (OptStats, Option<String>)> = BTreeMap::new();
    let ascii = crate::lower::bstr::enabled()
        .then(|| prepare_bstr_ascii(lcx, &mut funcs, &opt_cx, &mut prepared_meta));

    let mut lowered = Vec::with_capacity(lcx.names.len());
    let mut retry = false;
    for name in lcx.names {
        let Some(item) = lcx.table.get(name) else {
            continue;
        };
        let retargets = lcx.plan.call_retargets(name);
        let Some(mut input) =
            lower_input_with_retargets(name, item, lcx.ctx, lcx.plan.spill_names(), retargets)
        else {
            continue;
        };
        input.abi = Some(lcx.plan);
        input.final_lowering = true;
        input.bigints = lcx.bigints;
        input.nested_clone.map = lcx.repr.has_native_map;
        crate::profile::bump(crate::profile::Counter::LowerFinal);
        let mut stats = OptStats::default();
        let mut alloc = None;
        let mut result = match lcx.repr.bodies.get(name) {
            // A committed standard-operation item: the registry's native body replaces the `Func`
            // whole (Phase 4, D7), exactly like the `RCGJS_REPR_FORCE` harness.
            Some(body) => {
                funcs.remove(name);
                Ok(native_body_item(
                    name,
                    body,
                    (lcx.ctx.opts.optimizer != OptimizerMode::On)
                        .then_some(lcx.bigints)
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
                        None => prepare_opt_func(name, &mut func, input.interns, &opt_cx),
                    };
                    match prepared {
                        Err(decline) => Err(decline),
                        Ok((opt_stats, opt_alloc)) => {
                            stats = opt_stats;
                            alloc = opt_alloc;
                            crate::lower::lower_func_with_plans(
                                &input,
                                &func,
                                ascii.as_ref().map(|a| a.for_item(name)),
                                Some(lcx.soa),
                            )
                        }
                    }
                }
            },
        };
        if let Err(decline) = &result {
            if opt_debug_func().is_some_and(|filter| name.as_str().contains(filter)) {
                eprintln!("opt-func: {} declined: {decline}", name.as_str());
            }
        }
        let touched = lcx.plan.touches(name) || lcx.repr.touched.contains(name);
        if touched && lcx.fault.is_some_and(|item| name.as_str().contains(item)) {
            result = Err(Decline::Lower(LowerDecline::Other {
                op: OpClass::Region,
                detail: "JSC_JIR_ABI_FAULT".to_owned(),
            }));
        }
        if touched && repr_debug_solve() {
            if let Err(decline) = &result {
                eprintln!("lane-retry: `{}` declined: {decline:?}", name.as_str());
            }
        }
        let soa_touched = !lcx.soa.is_empty() && lcx.soa.item_may_touch_soa(item);
        retry |= (touched || soa_touched)
            && match &result {
                Err(_) => true,
                Ok(lowering) => refusal(
                    item,
                    lowering,
                    &refs_of(&lowering.decl, name),
                    lcx.defined,
                    lcx.live,
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
    body: &NativeBody,
    bigint_names: Option<&crate::lower::bigint::BigIntNames>,
) -> LoweredItem {
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
    LoweredItem {
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
/// which is the ordinary decline path — the whole run then rebuilds with an empty plan and prints
/// the baseline ABI. It can only ever make the link step keep *less* of a plan, so a build that
/// sets it by accident is slower, never wrong.
fn abi_fault() -> Option<String> {
    std::env::var("JSC_JIR_ABI_FAULT")
        .ok()
        .filter(|item| !item.is_empty())
}

/// Every item of the program by its [`crate::jir::ItemId`], with the JavaScript name and parameter
/// shape a host lowering needs to spell a call to it.
///
/// The index is built once per lane pass, before any item is replaced, for the same reason
/// `defined` is: the loop holds `&mut` on one item of the same table.
pub(super) fn item_index(table: &BTreeMap<JsName, LinkItem>) -> (ItemIndex, DropGlueIndex) {
    let mut index = ItemIndex::new();
    let mut drop_glue = DropGlueIndex::new();
    let (_, core_crate_ids) = crate::link::collect_panic_location_context(table);
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
            .and_then(|(body, interns)| crate::link::item_caller_location_def(body, interns));
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
                standard: match item.jir.body.as_ref().map(|body| body.semantic) {
                    Some(BodySemantic::Standard(operation)) => Some(operation),
                    _ => None,
                },
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

/// Every vtable item of the program by the `(concrete type, principal trait)` pair it is the
/// vtable of, which is what an unsizing coercion to `dyn` names (`lower/js/unsize.rs`).
///
/// Built the same way and for the same reason as [`item_index`]: v1 asked its namer for the name,
/// and the only thing that knows it here is the vtable item itself.
pub(super) fn vtable_index(table: &BTreeMap<JsName, LinkItem>) -> VtableIndex {
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
pub(super) fn apply(
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
    let opts = crate::opts::get();
    if opts.profile == Some(Profile::ReleaseSpeed) && opts.optimizer == OptimizerMode::On {
        crate::lower::js::post_opt::coalesce_disjoint_locals_decl(&mut lowered.decl);
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
        allocator_provider: Default::default(),
        jir: crate::item::JsItemJir::Synthesized,
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
pub(super) struct Applied {
    /// The bytes the lowering printed.
    pub(super) bytes: usize,
    /// The bytes it takes printed compact.
    pub(super) bytes_min: usize,
    /// Whether its linkable references differ from the baseline's.
    pub(super) refs_changed: bool,
}

/// Whether the driver may install `lowered` over `item`, and whether doing so moves what the item
/// reaches. `Err` is the message to report and the short name of the rule that refused it.
///
/// Split out of [`apply`] so phase C can ask the question before the table is touched: a lowering
/// the driver is going to refuse leaves the item on its baseline text, and for an item the ABI
/// plan rewrote that is the shape mismatch the retry exists to prevent.
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
        .filter(|name| defined.contains(*name) || crate::link::is_namer_name(name.as_str()))
        .collect()
}

/// Every free identifier `decl` mentions, its own name excluded: `JsItem::new`'s rule, which is
/// what the baseline's `refs` was built with.
pub(crate) fn refs_of(decl: &jsast::Stmt, name: &JsName) -> BTreeSet<JsName> {
    let mut refs = BTreeSet::new();
    jsast::visit_free_idents(decl, &mut |ident| {
        if ident != name.as_str() {
            refs.insert(JsName::new(ident.to_owned()));
        }
    });
    refs
}
