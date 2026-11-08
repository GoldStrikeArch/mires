//! Conservative native-Promise eligibility over post-`StateTransform` MIR.
//!
//! This pass only reports a lowerable graph. It never changes emitted JavaScript. A graph is
//! eligible only when every suspension is one canonical `.await` of another concrete coroutine or
//! of a compiler-owned awaitable ([`COMPILER_OWNED_AWAITABLES`]), and no state whose Rust behavior
//! depends on pinning, borrowing, pointers, wakers, or drop lives across a suspension point.

use std::collections::{BTreeSet, HashSet};

use rustc_hir::LangItem;
use rustc_middle::mir;
use rustc_middle::ty::{self, CoroutineArgsExt, Instance, Ty, TyCtxt};
use rustc_span::Symbol;

use crate::value::typing_env;

/// Awaitable types whose behavior this backend owns end to end, as
/// `crate::definition::path` strings.
///
/// Each wraps one host construct with settled semantics: a Promise registered
/// on first poll (`Settling`, `PromiseFuture`), a one shot browser task yield
/// (`YieldTask`), or an immediately ready value (`Ready`, from the `core` this
/// backend compiles). A native `async function` lowering would replace the
/// whole future with the host construct it wraps, so awaiting one keeps a
/// graph eligible, and holding one across its own await is not observable drop
/// state: none has a `Drop` impl, and the settling callbacks keep their own
/// reference to the shared state. The report records which of them a graph
/// awaits, so an emission round knows exactly what it has to cover.
const COMPILER_OWNED_AWAITABLES: &[&str] = &[
    "core::future::ready::Ready",
    "mires_js::promise::PromiseFuture",
    "mires_js::promise::YieldTask",
    "view_async::promise::Settling",
];

/// The report-only result for one async terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromiseEligibility {
    pub(crate) eligible: bool,
    pub(crate) coroutines: usize,
    pub(crate) suspension_points: usize,
    pub(crate) await_loops: usize,
    /// The [`COMPILER_OWNED_AWAITABLES`] the graph awaits.
    pub(crate) compiler_owned: BTreeSet<&'static str>,
    pub(crate) fallback: Option<&'static str>,
}

impl PromiseEligibility {
    fn pinned_owner() -> Self {
        Self {
            eligible: false,
            coroutines: 0,
            suspension_points: 0,
            await_loops: 0,
            compiler_owned: BTreeSet::new(),
            fallback: Some(Failure::OwnerAware.reason()),
        }
    }
}

/// Checks one terminal future without changing its selected poller lane.
pub(crate) fn promise_eligibility<'tcx>(
    tcx: TyCtxt<'tcx>,
    future: Ty<'tcx>,
    owner_aware: bool,
) -> PromiseEligibility {
    if owner_aware {
        return PromiseEligibility::pinned_owner();
    }

    let mut analysis = Analysis {
        tcx,
        seen: HashSet::new(),
        failures: BTreeSet::new(),
        coroutines: 0,
        suspension_points: 0,
        await_loops: 0,
        compiler_owned: BTreeSet::new(),
    };
    analysis.coroutine(future);
    PromiseEligibility {
        eligible: analysis.failures.is_empty(),
        coroutines: analysis.coroutines,
        suspension_points: analysis.suspension_points,
        await_loops: analysis.await_loops,
        compiler_owned: analysis.compiler_owned,
        fallback: analysis.failures.first().map(|failure| failure.reason()),
    }
}

struct Analysis<'tcx> {
    tcx: TyCtxt<'tcx>,
    seen: HashSet<Ty<'tcx>>,
    failures: BTreeSet<Failure>,
    coroutines: usize,
    suspension_points: usize,
    await_loops: usize,
    compiler_owned: BTreeSet<&'static str>,
}

impl<'tcx> Analysis<'tcx> {
    fn coroutine(&mut self, coroutine: Ty<'tcx>) {
        let ty::Coroutine(def_id, args) = *coroutine.kind() else {
            self.failures.insert(Failure::NotConcreteCoroutine);
            return;
        };
        if !self.seen.insert(coroutine) {
            self.failures.insert(Failure::RecursiveGraph);
            return;
        }
        self.coroutines += 1;

        if !self.tcx.is_mir_available(def_id) {
            self.failures.insert(Failure::MirUnavailable);
            return;
        }
        let Ok(layout) = self.tcx.coroutine_layout(def_id, args) else {
            self.failures.insert(Failure::LayoutUnavailable);
            return;
        };
        let suspensions = layout
            .variant_fields
            .len()
            .saturating_sub(ty::CoroutineArgs::RESERVED_VARIANTS);
        self.suspension_points += suspensions;

        if suspensions != 0 {
            let upvars = args.as_coroutine().tupled_upvars_ty();
            self.saved_type(upvars);
        }
        let instance = Instance::new_raw(def_id, args);
        let mut saved_coroutines = HashSet::new();
        for saved in &layout.field_tys {
            let saved = self.normalize(instance, saved.ty);
            if matches!(saved.kind(), ty::Coroutine(..)) {
                saved_coroutines.insert(saved);
            }
            self.saved_type(saved);
        }

        let body = self.tcx.instance_mir(instance.def);
        let mut awaits = 0;
        let mut polled_coroutines = HashSet::new();
        for data in body.basic_blocks.iter() {
            let mir::TerminatorKind::Call {
                func,
                args: call_args,
                destination,
                target,
                ..
            } = &data.terminator().kind
            else {
                continue;
            };
            let function = self.normalize(instance, func.ty(&body.local_decls, self.tcx));
            let ty::FnDef(called, generic_args) = *function.kind() else {
                if self.poll_result(function) {
                    self.failures.insert(Failure::DynamicPoll);
                }
                continue;
            };
            let Some(future_poll) = self.tcx.lang_items().future_poll_fn() else {
                self.failures.insert(Failure::FutureLangItemUnavailable);
                continue;
            };
            if called != future_poll {
                if call_args.iter().any(|argument| {
                    self.task_state_type(
                        self.normalize(instance, argument.node.ty(&body.local_decls, self.tcx)),
                    )
                }) {
                    self.failures.insert(Failure::ManualTaskState);
                }
                continue;
            }

            awaits += 1;
            self.await_loops += 1;
            if !target.is_some_and(|target| canonical_await(body, *destination, target)) {
                self.failures.insert(Failure::NonCanonicalAwait);
            }
            let Some(receiver) = call_args.first() else {
                self.failures.insert(Failure::DynamicPoll);
                continue;
            };
            let receiver = self.normalize(instance, receiver.node.ty(&body.local_decls, self.tcx));
            let Some(child) = self.pinned_future(receiver) else {
                self.failures.insert(Failure::DynamicPoll);
                continue;
            };

            let Some(bound_args) = generic_args.no_bound_vars() else {
                self.failures.insert(Failure::DynamicPoll);
                continue;
            };
            let resolved = Instance::try_resolve(self.tcx, typing_env(), called, bound_args)
                .ok()
                .flatten();
            if resolved.is_none() {
                self.failures.insert(Failure::DynamicPoll);
                continue;
            }
            if let Some(owned) = self.compiler_owned_awaitable(child) {
                self.compiler_owned.insert(owned);
                continue;
            }
            if !matches!(child.kind(), ty::Coroutine(..)) {
                self.failures.insert(Failure::CustomPoll);
                continue;
            }
            polled_coroutines.insert(child);
            self.coroutine(child);
        }
        if !saved_coroutines.is_subset(&polled_coroutines) {
            self.failures.insert(Failure::UnsupportedState);
        }
        if awaits != suspensions {
            self.failures.insert(Failure::SuspensionMismatch);
        }
    }

    fn normalize(&self, instance: Instance<'tcx>, ty: Ty<'tcx>) -> Ty<'tcx> {
        instance.instantiate_mir_and_normalize_erasing_regions(
            self.tcx,
            typing_env(),
            ty::EarlyBinder::bind(self.tcx, ty),
        )
    }

    fn saved_type(&mut self, ty: Ty<'tcx>) {
        if matches!(ty.kind(), ty::Coroutine(..)) {
            if ty.needs_drop(self.tcx, typing_env()) {
                self.failures.insert(Failure::DropState);
            }
            return;
        }
        // A compiler-owned awaitable lives in the state exactly while its own
        // await polls it, and dropping one is unobservable; see
        // [`COMPILER_OWNED_AWAITABLES`]. One nested inside another saved value
        // is not the canonical await shape and keeps the checks below.
        if let Some(owned) = self.compiler_owned_awaitable(ty) {
            self.compiler_owned.insert(owned);
            return;
        }
        if ty.needs_drop(self.tcx, typing_env()) {
            self.failures.insert(Failure::DropState);
            return;
        }
        self.walk_saved_type(ty);
    }

    /// The [`COMPILER_OWNED_AWAITABLES`] entry this type is, if any, matched
    /// by exact definition path rather than by name pattern.
    ///
    /// The path is assembled from the crate name and the raw definition path,
    /// which spells an item the same way whether the crate defining it is the
    /// one being compiled or a dependency.
    fn compiler_owned_awaitable(&self, ty: Ty<'tcx>) -> Option<&'static str> {
        let ty::Adt(definition, _) = *ty.kind() else {
            return None;
        };
        let identity = definition.did();
        let path = format!(
            "{}{}",
            self.tcx.crate_name(identity.krate),
            self.tcx.def_path(identity).to_string_no_crate_verbose()
        );
        COMPILER_OWNED_AWAITABLES
            .iter()
            .copied()
            .find(|&candidate| candidate == path)
    }

    fn walk_saved_type(&mut self, ty: Ty<'tcx>) {
        match *ty.kind() {
            ty::Ref(..) => {
                self.failures.insert(Failure::BorrowedState);
            }
            ty::RawPtr(..) => {
                self.failures.insert(Failure::PointerState);
            }
            ty::Adt(definition, arguments) => {
                if self.tcx.is_lang_item(definition.did(), LangItem::Pin) {
                    self.failures.insert(Failure::PinnedState);
                }
                if self
                    .tcx
                    .is_diagnostic_item(Symbol::intern("Waker"), definition.did())
                {
                    self.failures.insert(Failure::ManualTaskState);
                }
                for field in definition.all_fields() {
                    let field = self
                        .tcx
                        .normalize_erasing_regions(typing_env(), field.ty(self.tcx, arguments));
                    self.walk_saved_type(field);
                }
            }
            ty::Tuple(elements) => {
                for element in elements {
                    self.walk_saved_type(element);
                }
            }
            ty::Array(element, _) | ty::Slice(element) | ty::Pat(element, _) => {
                self.walk_saved_type(element);
            }
            ty::Dynamic(..) | ty::FnPtr(..) | ty::Foreign(..) => {
                self.failures.insert(Failure::UnsupportedState);
            }
            ty::Closure(..) | ty::CoroutineClosure(..) => {
                self.failures.insert(Failure::UnsupportedState);
            }
            ty::Coroutine(..) => {
                self.failures.insert(Failure::UnsupportedState);
            }
            _ => {}
        }
    }

    fn task_state_type(&self, ty: Ty<'tcx>) -> bool {
        match *ty.kind() {
            ty::Ref(_, inner, _) | ty::RawPtr(inner, _) => self.task_state_type(inner),
            ty::Adt(definition, arguments) => {
                self.tcx.is_lang_item(definition.did(), LangItem::Context)
                    || self
                        .tcx
                        .is_diagnostic_item(Symbol::intern("Waker"), definition.did())
                    || arguments
                        .types()
                        .any(|argument| self.task_state_type(argument))
            }
            ty::Tuple(elements) => elements.iter().any(|element| self.task_state_type(element)),
            ty::Array(element, _) | ty::Slice(element) | ty::Pat(element, _) => {
                self.task_state_type(element)
            }
            _ => false,
        }
    }

    fn pinned_future(&self, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
        let ty::Adt(definition, arguments) = *ty.kind() else {
            return None;
        };
        if !self.tcx.is_lang_item(definition.did(), LangItem::Pin) {
            return None;
        }
        match *arguments.type_at(0).kind() {
            ty::Ref(_, future, rustc_hir::Mutability::Mut) => Some(future),
            _ => None,
        }
    }

    fn poll_result(&self, function: Ty<'tcx>) -> bool {
        let signature = match *function.kind() {
            ty::FnPtr(signature, _) => signature.skip_binder(),
            _ => return false,
        };
        match *signature.output().kind() {
            ty::Adt(definition, _) => self.tcx.is_lang_item(definition.did(), LangItem::Poll),
            _ => false,
        }
    }
}

/// Checks the stable `Future::poll -> discriminant -> switch` shape produced for `.await`.
fn canonical_await<'tcx>(
    body: &mir::Body<'tcx>,
    destination: mir::Place<'tcx>,
    target: mir::BasicBlock,
) -> bool {
    let data = &body.basic_blocks[target];
    let mir::TerminatorKind::SwitchInt { discr, .. } = &data.terminator().kind else {
        return false;
    };
    let Some(discriminant_local) = discr.place().map(|place| place.local) else {
        return false;
    };
    data.statements.iter().any(|statement| {
        let mir::StatementKind::Assign(assignment) = &statement.kind else {
            return false;
        };
        assignment.0.as_local() == Some(discriminant_local)
            && matches!(&assignment.1, mir::Rvalue::Discriminant(place) if *place == destination)
    })
}

/// Stable priority for the one fallback reason written to reports.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Failure {
    OwnerAware,
    NotConcreteCoroutine,
    FutureLangItemUnavailable,
    MirUnavailable,
    LayoutUnavailable,
    RecursiveGraph,
    DynamicPoll,
    CustomPoll,
    NonCanonicalAwait,
    SuspensionMismatch,
    ManualTaskState,
    PinnedState,
    BorrowedState,
    PointerState,
    DropState,
    UnsupportedState,
}

impl Failure {
    const fn reason(self) -> &'static str {
        match self {
            Self::OwnerAware => "owner-aware Topcoat scheduling is a pinned async boundary",
            Self::NotConcreteCoroutine => "terminal future is not one concrete coroutine",
            Self::FutureLangItemUnavailable => {
                "the Rust Future::poll semantic identity is unavailable"
            }
            Self::MirUnavailable => "a concrete coroutine has no post-StateTransform MIR",
            Self::LayoutUnavailable => "a concrete coroutine has no saved-state layout",
            Self::RecursiveGraph => "the concrete coroutine graph is recursive",
            Self::DynamicPoll => "a future poll target or receiver is not statically known",
            Self::CustomPoll => "an awaited child uses a custom Future::poll implementation",
            Self::NonCanonicalAwait => {
                "a Future::poll call is not in a canonical post-StateTransform await loop"
            }
            Self::SuspensionMismatch => {
                "coroutine suspension points do not match canonical await loops"
            }
            Self::ManualTaskState => {
                "the coroutine observes Context or Waker outside canonical Future::poll"
            }
            Self::PinnedState => "pinned state remains live across a suspension point",
            Self::BorrowedState => "a Rust reference remains live across a suspension point",
            Self::PointerState => "a raw pointer remains live across a suspension point",
            Self::DropState => "observable drop state remains live across a suspension point",
            Self::UnsupportedState => "unsupported state remains live across a suspension point",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_priority_is_stable() {
        let failures = BTreeSet::from([
            Failure::DropState,
            Failure::CustomPoll,
            Failure::NonCanonicalAwait,
        ]);
        assert_eq!(
            failures.first().map(|failure| failure.reason()),
            Some("an awaited child uses a custom Future::poll implementation")
        );
    }

    #[test]
    fn owner_boundary_has_no_analyzed_graph() {
        let result = PromiseEligibility::pinned_owner();
        assert!(!result.eligible);
        assert_eq!(result.coroutines, 0);
        assert_eq!(
            result.fallback,
            Some("owner-aware Topcoat scheduling is a pinned async boundary")
        );
    }
}
