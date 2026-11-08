//! A `@click` on a row of a `for` loop: the two ways to wire a grid, and what each costs.
//!
//! Every handler fixture before this one sits on an element the view names once, so the closure it
//! carries captures only what the view's own scope holds. A grid does not have that shape: the thing
//! a click has to identify is the row, and the row exists only inside the loop. Both ways of writing
//! it work, and the fixture is what pins that.
//!
//! * `per_row` is pattern A: each row carries its own `@click` capturing the loop's binding, and
//!   each handler acts on the row it was built for. The environment is a function-scoped `let` the
//!   loop reassigns, so a thunk built inside a loop is emitted as
//!   `(($c) => ($a0) => f($c, $a0))(env)` -- an immediately invoked wrapper that gives every turn a
//!   binding of its own. See `abi.rs::closure_thunk`.
//! * `container` is pattern B: one `@click` on the element the rows sit in, and each row is a
//!   `<button value=(..)>`. The handler reads `e.target_value()`, which compiles to
//!   `event.target.value || ""` -- the element the event came FROM, not the one the listener sits on
//!   -- so a delegated click names its row without any closure per row at all. That is one handler
//!   for a whole grid rather than one per cell, which is what makes it the cheaper of the two.
//! * `per_row_reactive` is pattern A again, in a loop whose iterated expression reads a signal, so
//!   the rows are built inside the accessor `_$insert` was handed rather than once at setup. The
//!   wrapper is emitted there too, so the rows of a render each capture their own environment.
//!
//! No `.family`. The corpus's event family (04) is about which sink a handler reaches, and its
//! loop family (13) is a static list with no handlers; neither reference has a counterpart to a
//! handler that captures a row.
//!
//! The emission is pinned by `23_for_row_handlers.js.expected`. What the emission MEANS is pinned by
//! `scripts/for-handler-check.mjs`, which invokes the handlers with a fake event: the dom suite's
//! trace records that a handler was installed and never calls one, so which row a closure captured
//! is invisible to it. Same limitation, and same answer, as `21_event_accessors.rs` and
//! `22_reactive_for.rs`. The three views are exported so it can build one at a time.

#![no_std]
#![no_main]

use view_dom_macro::dom_view_client_only;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

struct Row {
    id: u32,
    name: &'static str,
}

/// The rows the loops draw from. A `static` rather than an argument, so the accessor the runtime
/// keeps re-running has nothing to borrow.
static ROWS: [Row; 3] = [
    Row { id: 10, name: "alpha" },
    Row { id: 20, name: "beta" },
    Row { id: 30, name: "gamma" },
];

/// Pattern A: one handler per row, capturing the row it was built for.
#[unsafe(no_mangle)]
pub fn per_row() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            signal picked = 0u32;
            <p>$(picked.get())</p>
            <ul>
                for row in ROWS.iter() {
                    <li><button @click=$(|_e| picked.set(row.id))>(row.name)</button></li>
                }
            </ul>
        </div>
    }
}

/// Pattern B: one handler on the container, and the row names itself in the button's value.
///
/// One listener for a whole grid rather than one per cell.
#[unsafe(no_mangle)]
pub fn container() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            signal picked = "";
            <p>$(picked.get())</p>
            <ul @click=$(|e| picked.set(e.target_value()))>
                for row in ROWS.iter() {
                    <li><button value=(row.name)>(row.name)</button></li>
                }
            </ul>
        </div>
    }
}

/// Pattern A in a list that re-renders, which is the shape a grid has.
///
/// The loop reads `limit`, so its hole is filled through an accessor and the rows -- and with them
/// the per-row closures -- are built again on every render. Each row of a render captures its own
/// environment, one nesting level down from `per_row`.
#[unsafe(no_mangle)]
pub fn per_row_reactive() -> view_abi::Node {
    dom_view_client_only! {
        <div>
            signal limit = 30u32;
            signal picked = 0u32;
            <p>$(picked.get())</p>
            <ul>
                for row in ROWS.iter().filter(|row| row.id <= limit.get()) {
                    <li><button @click=$(|_e| picked.set(row.id))>(row.name)</button></li>
                }
            </ul>
        </div>
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let _ = per_row();
    let _ = container();
    let _ = per_row_reactive();
}
