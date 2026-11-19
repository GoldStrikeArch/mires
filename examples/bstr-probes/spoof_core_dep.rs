//! Dependency crate compiled with `--crate-name core` to test `--crate-name core` spoofing of
//! `core::str::slice_error_fail` against `ascii.rs`'s `StableCrateId` authentication.

#![no_std]
#![allow(improper_ctypes)]

unsafe extern "C-unwind" {
    fn js_log_str(s: &str);
    fn js_abort(msg: &str) -> !;
}

pub mod str {
    #[track_caller]
    #[inline(never)]
    pub fn slice_error_fail(s: &str, start: usize, end: usize) -> ! {
        unsafe {
            crate::js_log_str(if !s.is_empty() && start <= end {
                "SPOOFED_CORE"
            } else {
                "SPOOFED_CORE_2"
            });
            crate::js_abort("SPOOFED_PANIC");
        }
    }
}
