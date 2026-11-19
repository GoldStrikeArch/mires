//! Blocker 1 lexical collision regression: a private user `mod str { fn slice_error_fail }` in a
//! crate sorting before `core` (`--crate-name aaa_collide`) must never hijack `&"é"[1..2]`.

#![no_std]
#![no_main]
#![allow(improper_ctypes)]

unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { js_abort("EXPECTED_PANIC") }
}

mod str {
    #[track_caller]
    #[inline(never)]
    pub fn slice_error_fail(s: &str, start: usize, end: usize) -> &'static str {
        if !s.is_empty() && start <= end {
            "HIJACKED"
        } else {
            "STILL_HIJACKED"
        }
    }
}

#[inline(never)]
fn substring(s: &str, start: usize, end: usize) -> &str {
    &s[start..end]
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    unsafe { js_log_str(str::slice_error_fail("hello", 1, 3)) };
    unsafe { js_log_str(substring("é", 1, 2)) };
}
