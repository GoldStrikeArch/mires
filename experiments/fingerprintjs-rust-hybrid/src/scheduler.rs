//! Collection scheduling. Hybrid keeps only the upstream `mapWithBreaks` boundary.

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

/// Runs the boundary scheduler between source boundaries.
pub async fn collection_break(last_release: &mut f64, interval: f64) {
    boundary_break(last_release, interval).await;
}

/// Boundary artifact: checkpoints are no-ops. Kept so callers compile unchanged.
#[allow(clippy::unused_async)]
pub async fn source_checkpoint(last_release: &mut f64) {
    let _ = last_release;
}

/// Name recorded in benchmark output for this compiled scheduler.
pub const fn scheduler_name() -> &'static str {
    "boundary"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_reports_its_selected_scheduler() {
        assert_eq!(scheduler_name(), "boundary");
        assert_eq!(LOOP_RELEASE_INTERVAL, 16.0);
    }
}
