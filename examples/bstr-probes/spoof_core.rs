//! Blocker 1 `--crate-name core` spoof regression: links `spoof_core_dep.rs` compiled with
//! `--crate-name core` containing an exact-signature `str::slice_error_fail(...) -> !` spoof,
//! keeps it live via `keep_spoof`, and verifies `&"é"[1..2]` still hits authentic `core` panic.

#![no_std]
#![no_main]
#![allow(improper_ctypes)]

extern crate spoof_core;

unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { js_abort("EXPECTED_PANIC") }
}

#[inline(never)]
fn substring(s: &str, start: usize, end: usize) -> &str {
    &s[start..end]
}

#[unsafe(no_mangle)]
pub fn keep_spoof() -> ! {
    spoof_core::str::slice_error_fail("hello", 1, 3)
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe { js_log_str(substring("é", 1, 2)) };
}
