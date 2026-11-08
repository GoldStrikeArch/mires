//! V3 host-contract slot facts, end to end (Phase 4, F2/F3): a `reads, accepts(host_string)`
//! slot keeps its edge a SINK — the value's region can still commit, fixed to the one carrier
//! the host declared — while a `retains, mutates` slot pins ITS edge only, and an unrelated
//! value in the same function still commits.
//!
//! Both `&String` slots below would pin by TYPE (`HostParameterClass::Pin`); the declared facts
//! are what separate them. The expected stdout is mode-independent: the baseline lane, the jir
//! lane and every optimizer mode must print exactly this, and `scripts/repr-report-test.sh`
//! additionally reads the jir sidecar's `repr` block over this fixture: the sink value commits
//! `host-string`, the pinned value declines `legality.must-equal-baseline`, and the unrelated
//! builder still commits.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::{String, ToString};
use mires_js::{js_bindings, js_export};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

js_bindings! {
    /// A `&str` slot is a sink by type in both lanes: the observable output channel.
    #[js(call = "console.log")]
    fn show(text: &str);

    /// The type alone would pin; the declaration says the host only READS a JS string, so the
    /// edge is a sink and the value's region stays committable (fixed to `host-string`).
    #[js(method = "valueOf")]
    fn observe(#[js(reads, accepts(host_string))] text: &String);

    /// The host keeps writing through the value: the edge pins, and nothing else does.
    #[js(method = "valueOf")]
    fn keep(#[js(retains, mutates)] text: &mut String);
}

#[js_export]
pub fn rust_entry() {
    // The sink slot: an immutable formatted string crosses it, and its region still commits.
    let label = 42i32.to_string();
    observe(&label);
    show(&label);

    // The pin slot: this value stays baseline...
    let mut kept = String::new();
    kept.push_str("kept");
    keep(&mut kept);
    show(&kept);

    // ...while an unrelated builder in the same function still commits.
    let mut other = String::new();
    other.push_str("other");
    other.push('!');
    show(&other);
}
