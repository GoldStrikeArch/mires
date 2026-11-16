//! The JavaScript side of `scripts/native-f64-differential.sh`'s call-shape check: each export
//! formats its argument through one shape of `301_f64_format_paths.rs` and sends the text to the
//! host, where the check reads it. The `String`s stay inside the program, so the native route
//! may carry them; the script proves it did.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "../core-tests/prelude.rs"]
mod prelude;

#[path = "301_f64_format_paths.rs"]
mod paths;

#[no_mangle]
pub fn f64_debug_line(value: f64) {
    let line = paths::debug_line("x", value);
    prelude::print_str(line.as_str());
}

#[no_mangle]
pub fn f64_text(value: f64) {
    let text = paths::text(value);
    prelude::print_str(text.as_str());
}

#[no_mangle]
pub fn f64_reciprocal_text(ratio: f64) {
    let text = paths::reciprocal_text(ratio);
    prelude::print_str(text.as_str());
}

#[no_mangle]
pub fn rust_entry() {}
