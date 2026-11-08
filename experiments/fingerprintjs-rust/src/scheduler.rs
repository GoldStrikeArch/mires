//! Collection scheduling shared by the two separately compiled variants.

use mires_js::yield_task;

use crate::date_now;

/// FingerprintJS's source-boundary budget in milliseconds.
pub const LOOP_RELEASE_INTERVAL: f64 = 16.0;

/// Yields at the same source boundary as upstream when the budget has elapsed.
pub async fn boundary_break(last_release: &mut f64, interval: f64) {
    let now = date_now();
    if now >= *last_release + interval {
        *last_release = now;
        yield_task().await;
    }
}

/// Runs the scheduler selected for this artifact between source boundaries.
pub async fn collection_break(last_release: &mut f64, interval: f64) {
    #[cfg(fingerprint_fiber)]
    {
        let _ = interval;
        source_checkpoint(last_release).await;
    }
    #[cfg(not(fingerprint_fiber))]
    boundary_break(last_release, interval).await;
}

/// A cooperative checkpoint used only by the fiber artifact.
///
/// The boundary artifact compiles this to a no-op. Callers place checkpoints
/// only between independent batches, after any listener or browser work has
/// already been armed.
#[allow(clippy::unused_async)]
pub async fn source_checkpoint(last_release: &mut f64) {
    #[cfg(fingerprint_fiber)]
    {
        let now = date_now();
        if now >= *last_release + 8.0 {
            *last_release = now;
            yield_task().await;
        }
    }
    #[cfg(not(fingerprint_fiber))]
    {
        let _ = last_release;
    }
}

/// Name recorded in benchmark output for this compiled scheduler.
pub const fn scheduler_name() -> &'static str {
    if cfg!(fingerprint_fiber) {
        "fiber"
    } else {
        "boundary"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_reports_its_selected_scheduler() {
        let expected = if cfg!(fingerprint_fiber) {
            "fiber"
        } else {
            "boundary"
        };
        assert_eq!(scheduler_name(), expected);
        assert_eq!(LOOP_RELEASE_INTERVAL, 16.0);
    }
}
