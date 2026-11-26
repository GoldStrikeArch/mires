//! Transactional per-function rewrite helper: clone -> mutate -> verify -> witness check ->
//! commit or rollback.

use crate::jir::repr::Func;
use crate::jir::TypeTable;

use super::observed_carriers;
use super::witness;

/// Outcome of a transactional per-function rewrite attempt.
#[derive(Debug)]
pub(crate) enum TxOutcome<T> {
    /// The mutation closure made no changes; the original function is untouched.
    Unchanged,
    /// The rewritten candidate passed carrier verification and witness validation.
    Committed { func: Func, payload: T },
    /// Verification or witness validation failed; the candidate was discarded.
    RolledBack,
}

/// Clones `func`, applies `mutate`, and validates the candidate with [`Func::verify_carriers`]
/// (under [`observed_carriers`]) and `check_witnesses` (when [`witness::enabled`] is true).
///
/// `mutate` returns `Some(payload)` when it made changes worth committing, or `None` when the
/// function was unchanged. `check_witnesses` runs only after verification succeeds and only when
/// witness checking is enabled.
pub(crate) fn try_rewrite_func<T>(
    func: &Func,
    table: &TypeTable,
    mutate: impl FnOnce(&mut Func) -> Option<T>,
    check_witnesses: impl FnOnce(&Func, &T) -> bool,
) -> TxOutcome<T> {
    let mut candidate = func.clone();
    let Some(payload) = mutate(&mut candidate) else {
        return TxOutcome::Unchanged;
    };
    if candidate
        .verify_carriers(table, observed_carriers(&candidate))
        .is_err()
    {
        return TxOutcome::RolledBack;
    }
    if witness::enabled() && !check_witnesses(&candidate, &payload) {
        return TxOutcome::RolledBack;
    }
    TxOutcome::Committed {
        func: candidate,
        payload,
    }
}
