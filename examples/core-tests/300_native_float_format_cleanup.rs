//! `f64` text at the two call shapes the idiomatic FingerprintJS port formats floats in:
//! `format!` of a default template over host `&str` arguments and an `f64` (`api::print_debug`),
//! and `f64::to_string` (`sources_dom::prepare_natural_fonts`), each sent to the host.
//!
//! Under `js-profile=release-balanced` (`js-cleanup=on`) both take the native `f64` route, which
//! keeps `core`'s `flt2dec` out of the program; `scripts/native-f64-differential.sh` proves the
//! route is taken and compares the same shapes against real Rust over its whole oracle. Every
//! text here is Rust's `Display`: no exponent at `1e21` or `1e-7`, `inf`/`NaN`/`-0`, the shortest
//! digits of an exact decimal midpoint, the subnormal and the largest finite extremes.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::ToString;

#[path = "prelude.rs"]
mod prelude;

const VERSION: &str = "5.2.0";

fn debug_line(name: &str, value: f64) {
    let message = alloc::format!(
        "version: {}\nname: {}\nvalue: {}\n```",
        VERSION,
        name,
        value
    );
    prelude::print_str(message.as_str());
}

fn reciprocal(ratio: f64) {
    let text = (1.0 / ratio).to_string();
    prelude::print_str(text.as_str());
}

#[no_mangle]
pub fn rust_entry() {
    let values = [
        0.0,
        -0.0,
        1.0,
        -2.5,
        0.1,
        1.0 / 3.0,
        1.00000762939453125,
        1e-7,
        1e-6,
        7.5e-11,
        1e20,
        1e21,
        123456789012345680000.0,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    for (index, value) in values.into_iter().enumerate() {
        let name = if index % 2 == 0 { "even" } else { "odd" };
        debug_line(name, value);
        reciprocal(value);
    }
}
