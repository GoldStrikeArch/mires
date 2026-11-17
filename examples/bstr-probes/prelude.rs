//! The host print shim of `examples/core-tests/prelude.rs`, without its panic handler (the probe
//! fixture prints where a panic happened, so that the native oracle can print the same line).

#![allow(dead_code)]

unsafe extern "C-unwind" {
    pub fn js_log_str(s: &str);
    pub fn js_abort(msg: &str) -> !;
}

pub fn print_str(s: &str) {
    unsafe { js_log_str(s) }
}
