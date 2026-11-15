//! Dependency-aware reuse of unchanged lowerings across whole-program cost trials.
//!
//! Both exact program measurements — the physical ABI economics (`link/lane.rs`,
//! `physical_program_metrics`, B2) and the representation planner's acceptance cost
//! (`plan/representation.rs`, `program_cost`, B1) — price a trial by lowering every function of the
//! program through [`crate::plan::cost::lower_item`], although one trial changes the lowering input
//! of only a handful of items. A [`Reuse`] store, scoped to one physical evaluation or one
//! representation solve, keeps each item's lowering at its **neutral input**: the input the item
//! has in every trial that leaves it alone. A later trial whose input for the item is provably the
//! neutral one gets that lowering back instead of a new one.
//!
//! Only the per-item lowering is reused. Reachability, link deduplication, ordering, printing,
//! final item renaming, the helper closure and compression still run over the whole program, so a
//! decision is computed from exactly the bytes a fresh lowering would produce.
//!
//! # What a lowering reads
//!
//! `cost::lower_item(input, func)` optimizes a clone of `func` (`opt::optimize`, optimizer forced
//! on) and lowers it (`lower::lower_func`: `fuse::plan`, `structure`, `js::function::lower`), then
//! takes the allocation term of the declaration. A trial installs the declaration shortened and
//! printed compact, with its refs, allocation sites, and direct and loop callees. Every input:
//!
//! * `name` — the item's key, the same in every trial.
//! * `summary`, `body`, `origins`, `interns`, `table` — from the trial table's entry for `name`.
//!   B2's trial table is the source table plus `AbiPlan::install_clones`, which replaces exactly
//!   the names in `abi.clones()`; B1's is the planner table plus the commit's clones.
//! * `func` — the source `Func`, unless a trial replaced or rewrote it: B2 installs clone
//!   functions (`abi.clones()`), then `AbiPlan::rewrite` rewrites a flat target's signature
//!   (exactly when `abi.target_signature(name, func.item)` answers) and the calls of every
//!   `abi.touches(name)` body. B1 additionally inserts the commit's functions.
//! * `opts`, `items`, `drop_glue`, `vtables` — from the context of the one evaluation or solve a
//!   store lives in; the trial forces the same optimizer and minifier fields every time.
//!   [`Reuse::begin_trial`] checks that every trial prices the same source table, functions and
//!   options.
//! * `abi` — `None` in B1. In B2 `Some(trial)`, read in exactly two places:
//!   `target_signature(name, func.item)` for the declaration's own physical signature, and
//!   `call_signature(callee)` for each `Callee::Direct` call the optimized function makes.
//! * `spills` — the trial's register names, spelled only by a `SpillRead`/`SpillWrite` op, by a
//!   flat target's own out-lanes, and by the result lanes of a call whose callee has a selected
//!   signature. Only `plan/abi_flat.rs` creates spill ops.
//! * `physical` — `None` in both measurements.
//! * `call_retargets` — `abi.call_retargets(name)` in B2, the commit's retargets in B1.
//! * Process-wide state — `opts::get()` (the printer's ECMAScript target), `RCGJS_REPR_FORCE`,
//!   `RCGJS_JIR_PASSES` and the witness switches — is fixed for the process; the lowering keeps no
//!   other state between calls.
//!
//! # The neutral input
//!
//! An item's input is neutral in a trial when none of the above differs from a trial that changes
//! nothing about it:
//!
//! * **B2** ([`physical_untouched`] and [`physical_calls_neutral`]): the item is not touched, not
//!   an installed clone, not retargeted and not a flat target, and none of the direct callees of
//!   its function — before or after the optimizer — has a selected signature in the trial.
//! * **B1** ([`representation_untouched`]): the commit neither replaces its function nor clones or
//!   retargets it, and the trial's ABI plan neither touches it nor targets it.
//! * **Both**: neither the function nor its optimized form contains a spill op, and the optimizer
//!   accepted the function without changing its item ([`Deps`]).
//!
//! Anything not proven neutral is lowered afresh.
//!
//! # Verification
//!
//! `RCGJS_COST_REUSE_VERIFY=1` lowers every reused item anyway and panics, naming the item and the
//! trial, unless the fresh lowering is identical — byte-equal printed declaration, equal refs,
//! allocation figures and callees, or the same decline. A mismatch is a bug in the rule above and
//! is never silently repaired.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use crate::analysis::js_alloc::JsAllocSites;
use crate::item::{JsName, LinkItem};
use crate::jir::decline::Decline;
use crate::jir::repr::{Func, OpKind};
use crate::jir::semantic::{Callee, ItemId};
use crate::jsast;
use crate::lower::{LowerInput, LoweredItem};
use crate::opt::OptStats;
use crate::opts::{OptimizerMode, Opts};
use crate::plan::abi_flat::AbiPlan;
use crate::plan::cost::MeasuredItem;
use crate::profile::{self, Counter};

/// Whether `RCGJS_COST_REUSE_VERIFY=1` is set. Read once per process.
fn verify() -> bool {
    static VERIFY: OnceLock<bool> = OnceLock::new();
    *VERIFY.get_or_init(|| std::env::var("RCGJS_COST_REUSE_VERIFY").is_ok_and(|value| value == "1"))
}

/// One function's lowering as a whole-program trial installs it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Lowering {
    /// The declaration with its locals shortened, printed compact: the item's trial `js`.
    pub js: String,
    /// Every identifier the declaration mentions, its own name excluded.
    pub refs: BTreeSet<JsName>,
    /// The declaration's allocation sites, those lexically in a loop, and its allocation term.
    pub alloc: (u64, u64, u64),
    /// The items the declaration calls directly.
    pub direct_calls: BTreeSet<JsName>,
    /// The items the declaration calls from inside a structured loop.
    pub loop_calls: BTreeSet<JsName>,
}

impl Lowering {
    /// The trial form of `lowered`. `alloc_term` prices the declaration's allocation sites.
    pub(crate) fn new(
        lowered: LoweredItem,
        alloc_term: impl FnOnce(&JsAllocSites) -> u64,
    ) -> Lowering {
        let sites = JsAllocSites::of(&lowered.decl);
        let names = |callees: &BTreeSet<String>| {
            callees
                .iter()
                .map(|callee| JsName::new(callee.clone()))
                .collect()
        };
        let mut decl = lowered.decl;
        crate::minify::shorten_locals_for_measurement(&mut decl);
        Lowering {
            js: jsast::program_to_string_styled_for_target(
                std::slice::from_ref(&decl),
                jsast::Style::Compact,
                crate::opts::get().ecma_target,
            ),
            refs: lowered.refs,
            alloc: (sites.total(), sites.in_loop(), alloc_term(&sites)),
            direct_calls: names(sites.direct_calls()),
            loop_calls: names(sites.loop_calls()),
        }
    }

    /// The trial form of a measured function, priced by the term its measurement carries.
    fn measured(measured: MeasuredItem) -> Lowering {
        let alloc_term = measured.alloc_term;
        Lowering::new(measured.lowered, |_| alloc_term)
    }

    /// The first field in which two lowerings differ, if any.
    fn mismatch(&self, other: &Lowering) -> Option<&'static str> {
        if self.js != other.js {
            Some("js")
        } else if self.refs != other.refs {
            Some("refs")
        } else if self.alloc != other.alloc {
            Some("alloc")
        } else if self.direct_calls != other.direct_calls {
            Some("direct_calls")
        } else if self.loop_calls != other.loop_calls {
            Some("loop_calls")
        } else {
            None
        }
    }
}

/// What one source function's lowering can read from a trial beyond its own entry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Deps {
    /// Every `Callee::Direct` item the function calls, before and after the optimizer: the
    /// lowering asks the trial's ABI plan for each one's selected signature. The union keeps the
    /// set complete without relying on which calls the optimizer keeps.
    callees: BTreeSet<ItemId>,
    /// Whether either form contains a `SpillRead`/`SpillWrite`, which spells a register name.
    spills: bool,
    /// Whether the optimizer accepted the function and kept its item, so `callees` and `spills`
    /// describe what is lowered.
    exact: bool,
}

impl Deps {
    /// The dependencies of `func` lowered through `input`, mirroring `cost::lower_item`'s
    /// optimizer step on a clone.
    fn of(input: &LowerInput<'_>, func: &Func) -> Deps {
        let mut deps = Deps::default();
        deps.scan(func);
        let mut optimized = func.clone();
        let mut opts = input.opts.clone();
        opts.optimizer = OptimizerMode::On;
        let optimized_ok = crate::opt::optimize_for_probe(
            &mut optimized,
            input.table,
            input.interns,
            &opts,
            &mut OptStats::default(),
        )
        .is_ok();
        deps.exact = optimized_ok && optimized.item == func.item;
        if deps.exact {
            deps.scan(&optimized);
        }
        deps
    }

    fn scan(&mut self, func: &Func) {
        for op in func.blocks.iter().flat_map(|block| &block.ops) {
            match &op.kind {
                OpKind::Call {
                    callee: Callee::Direct { item, .. },
                    ..
                } => {
                    self.callees.insert(*item);
                }
                OpKind::SpillRead(_) | OpKind::SpillWrite { .. } => self.spills = true,
                _ => {}
            }
        }
    }

    /// Whether the lowering reads nothing from a trial beyond what the callers check.
    fn reusable(&self) -> bool {
        self.exact && !self.spills
    }
}

/// The questions a lowering asks of a trial's ABI plan, as [`physical_untouched`] and its
/// siblings ask them.
pub(crate) trait TrialAbi {
    /// Whether the plan rewrites the calls `name` makes.
    fn touches(&self, name: &JsName) -> bool;
    /// Whether the plan installs `name` as a private clone.
    fn installs_clone(&self, name: &JsName) -> bool;
    /// Whether the plan retargets any direct call `name` makes.
    fn retargets(&self, name: &JsName) -> bool;
    /// Whether `name` is the body whose own physical signature the plan changes.
    fn targets(&self, name: &JsName, item: ItemId) -> bool;
    /// Whether a direct call to `item` is spelled with a selected physical signature.
    fn reshapes_calls_to(&self, item: ItemId) -> bool;
}

impl TrialAbi for AbiPlan {
    fn touches(&self, name: &JsName) -> bool {
        AbiPlan::touches(self, name)
    }

    fn installs_clone(&self, name: &JsName) -> bool {
        self.clones().contains_key(name)
    }

    fn retargets(&self, name: &JsName) -> bool {
        !self.call_retargets(name).is_empty()
    }

    fn targets(&self, name: &JsName, item: ItemId) -> bool {
        self.target_signature(name, item).is_some()
    }

    fn reshapes_calls_to(&self, item: ItemId) -> bool {
        self.call_signature(item).is_some()
    }
}

/// Whether physical trial `abi` leaves `name`'s table entry, function and call targets as the
/// source has them. `item` is the source function's item.
pub(crate) fn physical_untouched(abi: &impl TrialAbi, name: &JsName, item: ItemId) -> bool {
    !abi.touches(name)
        && !abi.installs_clone(name)
        && !abi.retargets(name)
        && !abi.targets(name, item)
}

/// Whether an untouched function's lowering reads the same answers from physical trial `abi` as
/// from any other trial that leaves its callees alone.
pub(crate) fn physical_calls_neutral(abi: &impl TrialAbi, deps: &Deps) -> bool {
    deps.reusable()
        && !deps
            .callees
            .iter()
            .any(|callee| abi.reshapes_calls_to(*callee))
}

/// Whether a representation trial leaves `name` alone: `committed` says whether the commit
/// replaces, clones or retargets it, and `abi` is the trial's own ABI plan.
pub(crate) fn representation_untouched(
    abi: &impl TrialAbi,
    committed: bool,
    name: &JsName,
    item: ItemId,
) -> bool {
    !committed && !abi.touches(name) && !abi.targets(name, item)
}

/// Which measurement a store serves, for its counters and messages.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Physical,
    Representation,
}

/// The neutral lowerings of one physical evaluation or one representation solve.
pub(crate) struct Reuse {
    kind: Kind,
    /// The addresses of the source table, functions and options the store was first used with.
    scope: Option<[usize; 3]>,
    trials: usize,
    deps: BTreeMap<JsName, Deps>,
    neutral: BTreeMap<JsName, Result<Lowering, Decline>>,
}

impl Reuse {
    /// A store for one `physical_cost::evaluate_groups`.
    pub(crate) fn physical() -> Reuse {
        Reuse::new(Kind::Physical)
    }

    /// A store for one representation solve.
    pub(crate) fn representation() -> Reuse {
        Reuse::new(Kind::Representation)
    }

    fn new(kind: Kind) -> Reuse {
        Reuse {
            kind,
            scope: None,
            trials: 0,
            deps: BTreeMap::new(),
            neutral: BTreeMap::new(),
        }
    }

    /// Starts one trial over the source `table` and `funcs` under `opts`.
    ///
    /// A neutral lowering is neutral relative to the source maps and options it was lowered from,
    /// so every trial of one store must price the same ones; anything else is a caller bug.
    pub(crate) fn begin_trial(
        &mut self,
        table: &BTreeMap<JsName, LinkItem>,
        funcs: &BTreeMap<JsName, Result<Func, Decline>>,
        opts: &Opts,
    ) {
        let scope = [
            std::ptr::from_ref(table) as usize,
            std::ptr::from_ref(funcs) as usize,
            std::ptr::from_ref(opts) as usize,
        ];
        let bound = *self.scope.get_or_insert(scope);
        assert_eq!(
            bound, scope,
            "a lowering-reuse store priced a second program"
        );
        self.trials += 1;
    }

    /// `name`'s lowering in physical trial `abi`. `input` and `func` are the trial's.
    pub(crate) fn lower_physical(
        &mut self,
        abi: &AbiPlan,
        name: &JsName,
        input: &LowerInput<'_>,
        func: &Func,
    ) -> Result<Lowering, Decline> {
        // Untouched, `func` is the source function, so its dependencies may be cached by name.
        let neutral = physical_untouched(abi, name, func.item)
            && physical_calls_neutral(abi, self.deps(name, input, func));
        self.lower(name, neutral, input, func, || describe_physical(abi))
    }

    /// `name`'s lowering in a representation trial. `committed` says whether the commit replaces,
    /// clones or retargets `name`; `abi` is the trial's ABI plan; `describe` names the trial.
    pub(crate) fn lower_representation(
        &mut self,
        abi: &AbiPlan,
        committed: bool,
        name: &JsName,
        input: &LowerInput<'_>,
        func: &Func,
        describe: impl FnOnce() -> String,
    ) -> Result<Lowering, Decline> {
        let neutral = representation_untouched(abi, committed, name, func.item)
            && self.deps(name, input, func).reusable();
        self.lower(name, neutral, input, func, describe)
    }

    fn deps(&mut self, name: &JsName, input: &LowerInput<'_>, func: &Func) -> &Deps {
        self.deps
            .entry(name.clone())
            .or_insert_with(|| profile::time("lowering_reuse.deps", || Deps::of(input, func)))
    }

    fn lower(
        &mut self,
        name: &JsName,
        neutral: bool,
        input: &LowerInput<'_>,
        func: &Func,
        describe: impl FnOnce() -> String,
    ) -> Result<Lowering, Decline> {
        let (lowered, reused) = match self.kind {
            Kind::Physical => (Counter::LowerPhysical, Counter::LowerPhysicalReused),
            Kind::Representation => (Counter::LowerReprProgram, Counter::LowerReprReused),
        };
        if neutral {
            if let Some(stored) = self.neutral.get(name) {
                profile::bump(reused);
                if verify() {
                    profile::bump(Counter::LowerReuseVerified);
                    let fresh = crate::plan::cost::lower_item(input, func).map(Lowering::measured);
                    let mismatch = match (stored, &fresh) {
                        (Ok(stored), Ok(fresh)) => stored.mismatch(fresh),
                        (Err(stored), Err(fresh)) => (stored != fresh).then_some("decline"),
                        _ => Some("outcome"),
                    };
                    if let Some(field) = mismatch {
                        panic!(
                            "lowering reuse: `{}` reused at its neutral input differs from a fresh \
                             lowering in `{field}` ({:?} trial {}: {})",
                            name.as_str(),
                            self.kind,
                            self.trials,
                            describe(),
                        );
                    }
                }
                return stored.clone();
            }
        }
        profile::bump(lowered);
        let lowering = crate::plan::cost::lower_item(input, func).map(Lowering::measured);
        if neutral {
            self.neutral.insert(name.clone(), lowering.clone());
        }
        lowering
    }
}

/// The flat callees of a physical trial, for a verification failure.
fn describe_physical(abi: &AbiPlan) -> String {
    let mut sources: Vec<&str> = abi
        .economic_groups()
        .into_iter()
        .flatten()
        .filter_map(|item| abi.source_name(item))
        .map(JsName::as_str)
        .collect();
    sources.sort_unstable();
    format!("flat {}", sources.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::semantic::DefinitionId;

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: n,
            },
            instantiation: 0,
        }
    }

    fn name(text: &str) -> JsName {
        JsName::new(text.to_owned())
    }

    /// A trial ABI plan by its answers.
    #[derive(Default)]
    struct Plan {
        touched: BTreeSet<JsName>,
        clones: BTreeSet<JsName>,
        retargeted: BTreeSet<JsName>,
        targets: BTreeSet<(JsName, ItemId)>,
        flat: BTreeSet<ItemId>,
    }

    impl TrialAbi for Plan {
        fn touches(&self, name: &JsName) -> bool {
            self.touched.contains(name)
        }

        fn installs_clone(&self, name: &JsName) -> bool {
            self.clones.contains(name)
        }

        fn retargets(&self, name: &JsName) -> bool {
            self.retargeted.contains(name)
        }

        fn targets(&self, name: &JsName, item: ItemId) -> bool {
            self.targets.contains(&(name.clone(), item))
        }

        fn reshapes_calls_to(&self, item: ItemId) -> bool {
            self.flat.contains(&item)
        }
    }

    fn deps(callees: &[ItemId]) -> Deps {
        Deps {
            callees: callees.iter().copied().collect(),
            spills: false,
            exact: true,
        }
    }

    fn physical_neutral(plan: &Plan, name: &JsName, item: ItemId, deps: &Deps) -> bool {
        physical_untouched(plan, name, item) && physical_calls_neutral(plan, deps)
    }

    /// `flat` (item 2) has a selected signature, so its caller `caller` is touched. `stray`
    /// calls it too without being touched — a representation clone is outside the ABI planner's
    /// caller scan — and `leaf` calls only item 3, which keeps its canonical signature.
    fn plan() -> Plan {
        Plan {
            touched: BTreeSet::from([name("flat"), name("caller")]),
            targets: BTreeSet::from([(name("flat"), item(2))]),
            flat: BTreeSet::from([item(2)]),
            ..Plan::default()
        }
    }

    #[test]
    fn an_untouched_item_calling_only_canonical_callees_is_reusable() {
        let plan = plan();
        assert!(physical_neutral(
            &plan,
            &name("leaf"),
            item(4),
            &deps(&[item(3)])
        ));
        assert!(physical_neutral(&plan, &name("leaf"), item(4), &deps(&[])));
    }

    #[test]
    fn an_item_calling_a_reshaped_callee_is_not_reusable_even_when_untouched() {
        let plan = plan();
        assert!(!plan.touches(&name("stray")));
        assert!(!physical_neutral(
            &plan,
            &name("stray"),
            item(5),
            &deps(&[item(3), item(2)])
        ));
    }

    #[test]
    fn a_touched_target_cloned_or_retargeted_item_is_not_reusable() {
        let mut plan = plan();
        plan.clones.insert(name("flat$1"));
        plan.retargeted.insert(name("redirected"));
        let none = deps(&[]);
        assert!(!physical_neutral(&plan, &name("caller"), item(1), &none));
        assert!(!physical_neutral(&plan, &name("flat"), item(2), &none));
        assert!(!physical_neutral(&plan, &name("flat$1"), item(2), &none));
        assert!(!physical_neutral(
            &plan,
            &name("redirected"),
            item(6),
            &none
        ));

        // A target is matched by name and item together, as `AbiPlan::target_signature` is.
        let mut target_only = Plan::default();
        target_only.targets.insert((name("own"), item(7)));
        assert!(!physical_neutral(
            &target_only,
            &name("own"),
            item(7),
            &none
        ));
        assert!(physical_neutral(&target_only, &name("own"), item(8), &none));
    }

    #[test]
    fn spill_ops_or_an_unproven_optimization_forbid_reuse() {
        let plan = Plan::default();
        let spills = Deps {
            spills: true,
            ..deps(&[])
        };
        let inexact = Deps {
            exact: false,
            ..deps(&[])
        };
        assert!(!physical_calls_neutral(&plan, &spills));
        assert!(!physical_calls_neutral(&plan, &inexact));
        // The representation store asks the same of a function's dependencies.
        assert!(!spills.reusable());
        assert!(!inexact.reusable());
        assert!(deps(&[item(2)]).reusable());
    }

    #[test]
    fn a_representation_trial_reuses_only_items_neither_the_commit_nor_its_plan_changes() {
        let plan = plan();
        // The representation measurement lowers without an ABI plan, so calling a flat callee
        // does not by itself change the lowering; the rewrite of a touched body does.
        assert!(representation_untouched(
            &plan,
            false,
            &name("stray"),
            item(5)
        ));
        assert!(representation_untouched(
            &plan,
            false,
            &name("leaf"),
            item(4)
        ));
        assert!(!representation_untouched(
            &plan,
            true,
            &name("leaf"),
            item(4)
        ));
        assert!(!representation_untouched(
            &plan,
            false,
            &name("caller"),
            item(1)
        ));
        assert!(!representation_untouched(
            &plan,
            false,
            &name("flat"),
            item(2)
        ));
    }
}
