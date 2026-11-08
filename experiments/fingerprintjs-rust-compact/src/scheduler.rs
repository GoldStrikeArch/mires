//! Boundary `mapWithBreaks` expressed with host Promises. No async/await.

use alloc::rc::Rc;
use core::cell::Cell;

use mires_js::{callback0, invoke, JsValue};

use crate::{array, call, date_now, promise, push, settle, yield_to_task};

/// FingerprintJS's source-boundary budget in milliseconds.
pub const LOOP_RELEASE_INTERVAL: f64 = 16.0;

/// Runs `each(index)` for `0..count`, yielding through `MessageChannel` when the budget elapses.
///
/// Matches upstream `mapWithBreaks`: the per-item callback is not awaited, and the returned
/// Promise resolves to the collected host array.
pub fn map_with_breaks(count: usize, interval: f64, each: JsValue) -> JsValue {
    let results = array();
    let last_release = Rc::new(Cell::new(date_now()));
    let index = Rc::new(Cell::new(0_usize));
    promise(move |resolve, _reject| {
        step(
            count,
            interval,
            each,
            results,
            last_release.clone(),
            index.clone(),
            resolve,
        );
        JsValue::undefined()
    })
}

fn step(
    count: usize,
    interval: f64,
    each: JsValue,
    results: JsValue,
    last_release: Rc<Cell<f64>>,
    index: Rc<Cell<usize>>,
    resolve: JsValue,
) {
    let current = index.get();
    if current >= count {
        settle(resolve, results);
        return;
    }

    let last_release_go = Rc::clone(&last_release);
    let index_go = Rc::clone(&index);
    let go = callback0(move || {
        push(
            results,
            invoke(each, JsValue::undefined(), &[JsValue::erase(current)]),
        );
        index_go.set(current + 1);
        step(
            count,
            interval,
            each,
            results,
            last_release_go.clone(),
            index_go.clone(),
            resolve,
        );
        JsValue::undefined()
    });

    let now = date_now();
    if now >= last_release.get() + interval {
        last_release.set(now);
        let _ = call(yield_to_task(), "then", &[go]);
    } else {
        let _ = invoke(go, JsValue::undefined(), &[]);
    }
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
