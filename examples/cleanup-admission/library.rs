#![no_std]
unsafe extern "C-unwind" { fn js_log_f64(value: f64); }
struct Guard;
impl Drop for Guard { fn drop(&mut self) { unsafe { js_log_f64(2.0) }; } }
#[inline(never)]
pub fn with_cleanup() { let _guard = Guard; unsafe { js_log_f64(1.0) }; }
